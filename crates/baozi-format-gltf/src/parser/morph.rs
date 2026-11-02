use super::*;
use baozi_import::authoring::{
    MorphDeltaAuthoring, MorphDeltaPayloadAuthoring, MorphDeltaSemantic,
    MorphTargetDefinitionAuthoring, MorphTargetIdentity, PrimitiveMorphTargetAuthoring,
};
use std::ops::Range;

pub(super) struct MorphContracts<'scope> {
    meshes: MeteredVec<'scope, Option<usize>>,
    cells: usize,
    definitions: usize,
    logical_meshes: usize,
    primitives: usize,
    targets: usize,
    deltas: usize,
    sample_work: u64,
}

pub(super) struct LogicalMorphStorage<'scope> {
    pub(super) definitions: AuthoringRange,
    pub(super) weights: MeteredVec<'scope, f32>,
}

pub(super) struct MorphTargetStorage {
    pub(super) deltas: Range<usize>,
}

pub(super) struct MorphDeltaStorage<'scope> {
    pub(super) semantic: MorphDeltaSemantic,
    pub(super) values: MeteredVec<'scope, f32>,
    pub(super) provenance: AccessorSources,
}

pub(super) struct MorphStorage<'scope> {
    pub(super) definitions: MeteredVec<'scope, MorphTargetDefinitionAuthoring<'static>>,
    pub(super) logical_meshes: MeteredVec<'scope, LogicalMorphStorage<'scope>>,
    pub(super) primitives: MeteredVec<'scope, Range<usize>>,
    pub(super) targets: MeteredVec<'scope, MorphTargetStorage>,
    pub(super) deltas: MeteredVec<'scope, MorphDeltaStorage<'scope>>,
}

fn add_cells(ctx: &mut ParseExecution<'_, '_>, cells: &mut usize, count: usize) -> Result<()> {
    *cells = {
        let argument_0 = ctx.limits().max_morph_cells;
        checked_add_with_limit(ctx, "max_morph_cells", *cells, count, argument_0)
    }?;
    Ok(())
}

pub(super) fn target_count(
    ctx: &mut ParseExecution<'_, '_>,
    mesh_index: usize,
    mesh: &json::mesh::Mesh<'_>,
) -> Result<usize> {
    let mut count = None;
    let work_group = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
    for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        if let Some(targets) = &primitive.targets {
            if targets.is_empty() {
                return Err(gltf_parse_error(format_args!(
                    "glTF mesh {mesh_index} primitive {primitive_index} has an empty targets array"
                )));
            }
            if count.is_some_and(|count| count != targets.len()) {
                return Err(gltf_parse_error(format_args!(
                    "glTF mesh {mesh_index} primitives disagree on morph target count"
                )));
            }
            count = Some(targets.len());
        }
    }
    Ok(count.unwrap_or(0))
}

fn validate_weights(
    ctx: &mut ParseExecution<'_, '_>,
    weights: &[f32],
    expected: usize,
    owner: impl fmt::Display,
) -> Result<()> {
    if weights.is_empty() || weights.len() != expected {
        return Err(gltf_parse_error(format_args!(
            "{owner} morph weight count must equal {expected}"
        )));
    }
    let work_group = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
    for chunk in weights.chunks(PARSER_POLL_INTERVAL) {
        ctx.with_work(
            &work_group,
            [to_u64(chunk.len())],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        if chunk.iter().any(|value| !value.is_finite()) {
            return Err(gltf_parse_error(format_args!(
                "{owner} morph weights must be finite"
            )));
        }
    }
    Ok(())
}

type MorphAccessor = (Semantic, MorphDeltaSemantic, Option<json::Index>);

fn target_accessors(target: &json::mesh::MorphTarget<'_>) -> [MorphAccessor; 3] {
    [
        (
            Semantic::Positions,
            MorphDeltaSemantic::Position,
            target.positions,
        ),
        (
            Semantic::Normals,
            MorphDeltaSemantic::Normal,
            target.normals,
        ),
        (
            Semantic::Tangents,
            MorphDeltaSemantic::Tangent,
            target.tangents,
        ),
    ]
}

pub(super) fn preflight<'scope>(
    ctx: &mut ParseExecution<'_, 'scope>,
    document: &crate::document::Document<'_>,
    closure: &ResourceClosure<'_>,
    geometry: &GeometryContracts<'_>,
    selected_nodes: &[bool],
) -> Result<MorphContracts<'scope>> {
    let root = document.tables();
    let mut contracts = MorphContracts {
        meshes: try_vec_capacity(
            ctx,
            ImportStorageClass::FormatNormalization,
            root.meshes.len(),
            "gltf-morph-mesh-contracts",
        )?,
        cells: 0,
        definitions: 0,
        logical_meshes: 0,
        primitives: 0,
        targets: 0,
        deltas: 0,
        sample_work: 0,
    };
    let work_group = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
    for (mesh_index, mesh) in root.meshes.iter().enumerate() {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        if !closure.meshes.as_slice()[mesh_index] {
            contracts.meshes.try_push_reserved(None)?;
            continue;
        }
        let width = target_count(ctx, mesh_index, mesh)?;
        contracts.logical_meshes += 1;
        if let Some(weights) = &mesh.weights {
            validate_weights(ctx, weights, width, format_args!("glTF mesh {mesh_index}"))?;
        }
        {
            let argument_0 = width
                .checked_mul(2)
                .ok_or_else(|| ctx.allocation_failure("gltf-morph-definition-count"))?;
            add_cells(ctx, &mut contracts.cells, argument_0)
        }?;
        contracts.definitions = contracts
            .definitions
            .checked_add(width)
            .ok_or_else(|| ctx.allocation_failure("gltf-morph-definition-count"))?;
        let geometry = geometry.meshes.as_slice()[mesh_index]
            .as_ref()
            .ok_or_else(|| gltf_parse_error("glTF morph geometry contracts are missing"))?;
        for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
            ctx.with_work(
                &work_group,
                [1],
                |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
            )?;
            contracts.primitives = contracts
                .primitives
                .checked_add(1)
                .ok_or_else(|| ctx.allocation_failure("gltf-morph-primitive-count"))?;
            contracts.targets = contracts
                .targets
                .checked_add(width)
                .ok_or_else(|| ctx.allocation_failure("gltf-morph-target-count"))?;
            add_cells(ctx, &mut contracts.cells, width)?;
            let vertex_count = geometry.primitives.as_slice()[primitive_index].position_count;
            for target in primitive.targets.iter().flatten() {
                ctx.with_work(
                    &work_group,
                    [1],
                    |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
                )?;
                if let Some((semantic, _)) = target.other_attributes.first_key_value() {
                    return Err(GltfFailure::unsupported("morph target attribute", format_args!("morph attribute {semantic} is outside the POSITION/NORMAL/TANGENT profile")).into());
                }
                for (semantic, kind, accessor) in target_accessors(target) {
                    let Some(accessor) = accessor else { continue };
                    ctx.with_work(
                        &work_group,
                        [1],
                        |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
                    )?;
                    if !primitive.attributes.contains_key(&semantic) {
                        return Err(gltf_parse_error(format_args!(
                            "glTF mesh {mesh_index} primitive {primitive_index} morph {kind:?} has no corresponding base attribute"
                        )));
                    }
                    let info = accessor::validate_mesh_attribute(
                        ctx,
                        document,
                        accessor.value(),
                        AccessorOwner::Primitive {
                            mesh_index,
                            primitive_index,
                        },
                        MeshAttributeUse::Morph(semantic),
                    )?;
                    if info.count != vertex_count {
                        return Err(gltf_parse_error(format_args!(
                            "glTF mesh {mesh_index} primitive {primitive_index} morph accessor count must equal {vertex_count}"
                        )));
                    }
                    let scalars = vertex_count
                        .checked_mul(3)
                        .ok_or_else(|| ctx.allocation_failure("gltf-morph-value-count"))?;
                    add_cells(ctx, &mut contracts.cells, scalars)?;
                    contracts.sample_work = to_u64(scalars)
                        .checked_mul(accessor::MESH_SAMPLE_WORK_PER_LANE)
                        .and_then(|work| contracts.sample_work.checked_add(work))
                        .ok_or_else(|| ctx.allocation_failure("gltf-morph-sample-work"))?;
                    contracts.deltas = contracts
                        .deltas
                        .checked_add(1)
                        .ok_or_else(|| ctx.allocation_failure("gltf-morph-delta-count"))?;
                }
            }
        }
        contracts.meshes.try_push_reserved(Some(width))?;
    }
    // Overrides borrow the admitted backend's immutable arrays. Count them before any resource IO.
    let mut override_cells = 0;
    for (node_index, node) in root.nodes.iter().enumerate() {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        if !selected_nodes[node_index] {
            continue;
        }
        if let Some(weights) = &node.weights {
            let mesh = node.mesh.ok_or_else(|| {
                gltf_parse_error(format_args!(
                    "glTF node {node_index} has morph weights without a mesh"
                ))
            })?;
            let width = contracts
                .meshes
                .as_slice()
                .get(mesh.value())
                .copied()
                .flatten()
                .ok_or_else(|| gltf_parse_error("glTF morph mesh was not selected"))?;
            validate_weights(ctx, weights, width, format_args!("glTF node {node_index}"))?;
            add_cells(ctx, &mut override_cells, weights.len())?;
        }
    }
    let mut total = contracts.cells;
    add_cells(ctx, &mut total, override_cells)?;
    Ok(contracts)
}

pub(super) fn build_storage<'scope>(
    ctx: &mut ParseExecution<'_, 'scope>,
    document: &crate::document::Document<'_>,
    buffers: &[Option<BufferData<'_>>],
    geometry: &GeometryContracts<'_>,
    contracts: &MorphContracts<'_>,
) -> Result<MorphStorage<'scope>> {
    let work_units = contracts
        .meshes
        .len()
        .checked_add(contracts.primitives)
        .and_then(|count| count.checked_add(contracts.deltas))
        .and_then(|count| count.checked_add(contracts.cells))
        .ok_or_else(|| ctx.allocation_failure("gltf-morph-work"))?;
    let work_units = to_u64(work_units)
        .checked_add(contracts.sample_work)
        .ok_or_else(|| ctx.allocation_failure("gltf-morph-work"))?;
    let provenance_work = to_u64(contracts.deltas)
        .checked_mul(AccessorSources::COLLECTION_WORK_BOUND + 16)
        .ok_or_else(|| ctx.allocation_failure("gltf-morph-provenance-work"))?;
    ctx.charge_work(provenance_work)?;
    ctx.with_work_group(
        [
            (ImportWorkClass::WorkUnits, work_units),
            (ImportWorkClass::MorphCells, to_u64(contracts.cells)),
        ],
        |ctx, work| -> Result<_> {
            let class = ImportStorageClass::SceneConstruction;
            let mut storage = MorphStorage {
                definitions: try_vec_capacity(
                    ctx,
                    class,
                    contracts.definitions,
                    "gltf-morph-definitions",
                )?,
                logical_meshes: try_vec_capacity(
                    ctx,
                    class,
                    contracts.logical_meshes,
                    "gltf-logical-morphs",
                )?,
                primitives: try_vec_capacity(
                    ctx,
                    class,
                    contracts.primitives,
                    "gltf-primitive-morphs",
                )?,
                targets: try_vec_capacity(ctx, class, contracts.targets, "gltf-morph-targets")?,
                deltas: try_vec_capacity(ctx, class, contracts.deltas, "gltf-morph-deltas")?,
            };
            for mesh in document.meshes() {
                ctx.poll()?;
                work.complete([1, 0])?;
                let Some(width) = contracts.meshes.as_slice()[mesh.index()] else {
                    continue;
                };
                let offset = authoring_u32(
                    ctx,
                    storage.definitions.len(),
                    "gltf-morph-definition-offset",
                )?;
                let mut weights =
                    try_vec_capacity(ctx, class, width, "gltf-morph-default-weights")?;
                for index in 0..width {
                    if index.is_multiple_of(PARSER_POLL_INTERVAL) {
                        ctx.poll()?;
                    }
                    storage
                        .definitions
                        .try_push_reserved(MorphTargetDefinitionAuthoring::new(
                            MorphTargetIdentity::new(authoring_u32(
                                ctx,
                                index,
                                "gltf-morph-identity",
                            )?),
                            Presence::Absent,
                        ))?;
                    weights
                        .try_push_reserved(mesh.weights().map_or(0.0, |weights| weights[index]))?;
                    work.complete([2, 2])?;
                }
                storage
                    .logical_meshes
                    .try_push_reserved(LogicalMorphStorage {
                        definitions: AuthoringRange::new(
                            offset,
                            authoring_u32(ctx, width, "gltf-morph-definition-width")?,
                        ),
                        weights,
                    })?;
                let mesh_geometry = geometry.meshes.as_slice()[mesh.index()]
                    .as_ref()
                    .ok_or_else(|| gltf_parse_error("glTF morph geometry is missing"))?;
                for primitive in mesh.primitives() {
                    ctx.poll()?;
                    work.complete([1, 0])?;
                    let start = storage.targets.len();
                    let source_targets =
                        json_primitive(ctx, document, mesh.index(), primitive.index())?
                            .targets
                            .as_deref()
                            .unwrap_or(&[]);
                    let vertices =
                        mesh_geometry.primitives.as_slice()[primitive.index()].position_count;
                    for target_index in 0..width {
                        if target_index.is_multiple_of(PARSER_POLL_INTERVAL) {
                            ctx.poll()?;
                        }
                        let delta_start = storage.deltas.len();
                        if let Some(target) = source_targets.get(target_index) {
                            for (attribute_semantic, semantic, accessor) in target_accessors(target)
                            {
                                let Some(accessor) = accessor else { continue };
                                let accessor =
                                    document.accessors().nth(accessor.value()).ok_or_else(
                                        || gltf_parse_error("glTF morph accessor disappeared"),
                                    )?;
                                let mut bounds = accessor::MeshAccessorBounds::<3>::new(
                                    accessor.row(),
                                    accessor.index(),
                                    attribute_semantic == Semantic::Positions,
                                )?;
                                let provenance = accessor_provenance(ctx, buffers, accessor)?;
                                let values = accessor::bind(ctx, accessor, buffers)?
                                    .samples::<3>()
                                    .ok_or_else(|| {
                                        GltfFailure::invalid_accessor(
                                            Some(accessor.index()),
                                            "invalid morph accessor shape",
                                        )
                                    })?;
                                let count = vertices.checked_mul(3).ok_or_else(|| {
                                    ctx.allocation_failure("gltf-morph-value-count")
                                })?;
                                let mut output =
                                    try_vec_capacity(ctx, class, count, "gltf-morph-delta-values")?;
                                for sample in values {
                                    if output.len().is_multiple_of(PARSER_POLL_INTERVAL) {
                                        ctx.poll()?;
                                    }
                                    if output.len() >= count {
                                        return Err(GltfFailure::invalid_accessor(
                                            Some(accessor.index()),
                                            "glTF morph delta exceeds its admitted count",
                                        )
                                        .into());
                                    }
                                    bounds.observe(&sample)?;
                                    output.try_extend_from_slice_reserved(&sample.decoded)?;
                                    work.complete([
                                        3 * (1 + accessor::MESH_SAMPLE_WORK_PER_LANE),
                                        3,
                                    ])?;
                                }
                                if output.len() != count {
                                    return Err(gltf_parse_error(
                                        "glTF morph delta value count changed after preflight",
                                    ));
                                }
                                bounds.finish()?;
                                storage.deltas.try_push_reserved(MorphDeltaStorage {
                                    semantic,
                                    values: output,
                                    provenance,
                                })?;
                                work.complete([1, 0])?;
                            }
                        }
                        storage.targets.try_push_reserved(MorphTargetStorage {
                            deltas: delta_start..storage.deltas.len(),
                        })?;
                        work.complete([1, 1])?;
                    }
                    storage
                        .primitives
                        .try_push_reserved(start..storage.targets.len())?;
                }
            }
            Ok(storage)
        },
    )
}

pub(super) fn delta_authoring<'a>(delta: &'a MorphDeltaStorage<'_>) -> MorphDeltaAuthoring<'a> {
    MorphDeltaAuthoring::new(
        delta.semantic,
        MorphDeltaPayloadAuthoring::Dense {
            values: delta.values.as_slice(),
        },
    )
}

pub(super) fn target_authoring<'a>(
    target: &MorphTargetStorage,
    deltas: &'a [MorphDeltaAuthoring<'a>],
) -> PrimitiveMorphTargetAuthoring<'a> {
    PrimitiveMorphTargetAuthoring::new(&deltas[target.deltas.clone()])
}
