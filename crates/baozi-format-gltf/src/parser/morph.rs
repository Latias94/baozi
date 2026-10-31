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
    pub(super) provenance: u32,
}

pub(super) struct MorphStorage<'scope> {
    pub(super) definitions: MeteredVec<'scope, MorphTargetDefinitionAuthoring<'static>>,
    pub(super) logical_meshes: MeteredVec<'scope, LogicalMorphStorage<'scope>>,
    pub(super) primitives: MeteredVec<'scope, Range<usize>>,
    pub(super) targets: MeteredVec<'scope, MorphTargetStorage>,
    pub(super) deltas: MeteredVec<'scope, MorphDeltaStorage<'scope>>,
}

fn add_cells(ctx: &ParseScope<'_>, cells: &mut usize, count: usize) -> Result<()> {
    *cells = checked_add_with_limit(
        ctx,
        "max_morph_cells",
        *cells,
        count,
        ctx.limits().max_morph_cells,
    )?;
    Ok(())
}

pub(super) fn target_count(
    ctx: &ParseScope<'_>,
    mesh_index: usize,
    mesh: &json::mesh::Mesh,
) -> Result<usize> {
    let mut count = None;
    for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
        ctx.charge_work(1)?;
        if let Some(targets) = &primitive.targets {
            if targets.is_empty() {
                return Err(gltf_parse_error(
                    ctx,
                    format!(
                        "glTF mesh {mesh_index} primitive {primitive_index} has an empty targets array"
                    ),
                ));
            }
            if count.is_some_and(|count| count != targets.len()) {
                return Err(gltf_parse_error(
                    ctx,
                    format!("glTF mesh {mesh_index} primitives disagree on morph target count"),
                ));
            }
            count = Some(targets.len());
        }
    }
    Ok(count.unwrap_or(0))
}

fn validate_weights(
    ctx: &ParseScope<'_>,
    weights: &[f32],
    expected: usize,
    owner: impl fmt::Display,
) -> Result<()> {
    if weights.is_empty() || weights.len() != expected {
        return Err(gltf_parse_error(
            ctx,
            format!("{owner} morph weight count must equal {expected}"),
        ));
    }
    for chunk in weights.chunks(PARSER_POLL_INTERVAL) {
        ctx.charge_work(to_u64(chunk.len()))?;
        if chunk.iter().any(|value| !value.is_finite()) {
            return Err(gltf_parse_error(
                ctx,
                format!("{owner} morph weights must be finite"),
            ));
        }
    }
    Ok(())
}

type MorphAccessor = (
    Semantic,
    MorphDeltaSemantic,
    Option<json::Index<json::accessor::Accessor>>,
);

fn target_accessors(target: &json::mesh::MorphTarget) -> [MorphAccessor; 3] {
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

fn validate_position_bounds(
    ctx: &ParseScope<'_>,
    accessor: &json::accessor::Accessor,
    index: usize,
) -> Result<([f32; 3], [f32; 3])> {
    let vector = |value: Option<&serde_json::Value>| -> Option<[f32; 3]> {
        let values = value?.as_array()?;
        let [x, y, z] = values.as_slice() else {
            return None;
        };
        let vector = [x.as_f64()? as f32, y.as_f64()? as f32, z.as_f64()? as f32];
        vector
            .iter()
            .all(|value| value.is_finite())
            .then_some(vector)
    };
    let bounds = vector(accessor.min.as_ref()).zip(vector(accessor.max.as_ref()));
    let Some((min, max)) =
        bounds.filter(|(min, max)| min.iter().zip(max).all(|(min, max)| *min <= *max))
    else {
        return Err(gltf_parse_error(
            ctx,
            format!(
                "glTF morph POSITION accessor {index} requires ordered finite VEC3 min and max"
            ),
        ));
    };
    Ok((min, max))
}

pub(super) fn preflight<'scope>(
    ctx: &'scope ParseScope<'_>,
    document: &gltf::Document,
    closure: &ResourceClosure<'_>,
    geometry: &GeometryContracts<'_>,
    selected_nodes: &[bool],
) -> Result<MorphContracts<'scope>> {
    let root = document.as_json();
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
    };
    for (mesh_index, mesh) in root.meshes.iter().enumerate() {
        ctx.charge_work(1)?;
        if !closure.meshes.as_slice()[mesh_index] {
            contracts.meshes.try_push_reserved(None)?;
            continue;
        }
        let width = target_count(ctx, mesh_index, mesh)?;
        contracts.logical_meshes += 1;
        if let Some(weights) = &mesh.weights {
            validate_weights(ctx, weights, width, format_args!("glTF mesh {mesh_index}"))?;
        }
        add_cells(
            ctx,
            &mut contracts.cells,
            width
                .checked_mul(2)
                .ok_or_else(|| ctx.allocation_failure("gltf-morph-definition-count"))?,
        )?;
        contracts.definitions = contracts
            .definitions
            .checked_add(width)
            .ok_or_else(|| ctx.allocation_failure("gltf-morph-definition-count"))?;
        let geometry = geometry.meshes.as_slice()[mesh_index]
            .as_ref()
            .ok_or_else(|| gltf_parse_error(ctx, "glTF morph geometry contracts are missing"))?;
        for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
            ctx.charge_work(1)?;
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
                ctx.charge_work(1)?;
                if let Some((semantic, _)) = target.other_attributes.first_key_value() {
                    return Err(GltfFailure::unsupported("morph target attribute", format_args!("morph attribute {semantic} is outside the POSITION/NORMAL/TANGENT profile")).into());
                }
                for (semantic, kind, accessor) in target_accessors(target) {
                    let Some(accessor) = accessor else { continue };
                    ctx.charge_work(1)?;
                    if !primitive.attributes.contains_key(&Checked::Valid(semantic)) {
                        return Err(gltf_parse_error(
                            ctx,
                            format!(
                                "glTF mesh {mesh_index} primitive {primitive_index} morph {kind:?} has no corresponding base attribute"
                            ),
                        ));
                    }
                    let info = validate_accessor(
                        ctx,
                        document,
                        accessor.value(),
                        AccessorOwner::Primitive {
                            mesh_index,
                            primitive_index,
                        },
                        "morph delta",
                        AccessorContract::vertex(
                            &[json::accessor::ComponentType::F32],
                            &[json::accessor::Type::Vec3],
                        ),
                    )?;
                    if info.count != vertex_count {
                        return Err(gltf_parse_error(
                            ctx,
                            format!(
                                "glTF mesh {mesh_index} primitive {primitive_index} morph accessor count must equal {vertex_count}"
                            ),
                        ));
                    }
                    if kind == MorphDeltaSemantic::Position {
                        validate_position_bounds(
                            ctx,
                            &root.accessors[accessor.value()],
                            accessor.value(),
                        )?;
                    }
                    let scalars = vertex_count
                        .checked_mul(3)
                        .ok_or_else(|| ctx.allocation_failure("gltf-morph-value-count"))?;
                    add_cells(ctx, &mut contracts.cells, scalars)?;
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
        ctx.charge_work(1)?;
        if !selected_nodes[node_index] {
            continue;
        }
        if let Some(weights) = &node.weights {
            let mesh = node.mesh.ok_or_else(|| {
                gltf_parse_error(
                    ctx,
                    format!("glTF node {node_index} has morph weights without a mesh"),
                )
            })?;
            let width = contracts
                .meshes
                .as_slice()
                .get(mesh.value())
                .copied()
                .flatten()
                .ok_or_else(|| gltf_parse_error(ctx, "glTF morph mesh was not selected"))?;
            validate_weights(ctx, weights, width, format_args!("glTF node {node_index}"))?;
            add_cells(ctx, &mut override_cells, weights.len())?;
        }
    }
    let mut total = contracts.cells;
    add_cells(ctx, &mut total, override_cells)?;
    Ok(contracts)
}

pub(super) fn build_storage<'scope>(
    ctx: &'scope ParseScope<'_>,
    document: &gltf::Document,
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
    ctx.with_work_group([
        (ImportWorkClass::WorkUnits, to_u64(work_units)),
        (ImportWorkClass::MorphCells, to_u64(contracts.cells)),
    ], |work| -> Result<_> {
        let class = ImportStorageClass::SceneConstruction;
        let mut storage = MorphStorage {
            definitions: try_vec_capacity(ctx, class, contracts.definitions, "gltf-morph-definitions")?,
            logical_meshes: try_vec_capacity(ctx, class, contracts.logical_meshes, "gltf-logical-morphs")?,
            primitives: try_vec_capacity(ctx, class, contracts.primitives, "gltf-primitive-morphs")?,
            targets: try_vec_capacity(ctx, class, contracts.targets, "gltf-morph-targets")?,
            deltas: try_vec_capacity(ctx, class, contracts.deltas, "gltf-morph-deltas")?,
        };
        for mesh in document.meshes() {
            work.poll()?;
            work.complete([1, 0])?;
            let Some(width) = contracts.meshes.as_slice()[mesh.index()] else { continue };
            let offset = authoring_u32(ctx, storage.definitions.len(), "gltf-morph-definition-offset")?;
            let mut weights = try_vec_capacity(ctx, class, width, "gltf-morph-default-weights")?;
            for index in 0..width {
                if index.is_multiple_of(PARSER_POLL_INTERVAL) { work.poll()?; }
                storage.definitions.try_push_reserved(MorphTargetDefinitionAuthoring::new(MorphTargetIdentity::new(authoring_u32(ctx, index, "gltf-morph-identity")?), Presence::Absent))?;
                weights.try_push_reserved(mesh.weights().map_or(0.0, |weights| weights[index]))?;
                work.complete([2, 2])?;
            }
            storage.logical_meshes.try_push_reserved(LogicalMorphStorage {
                definitions: AuthoringRange::new(offset, authoring_u32(ctx, width, "gltf-morph-definition-width")?),
                weights,
            })?;
            let mesh_geometry = geometry.meshes.as_slice()[mesh.index()].as_ref().ok_or_else(|| gltf_parse_error(ctx, "glTF morph geometry is missing"))?;
            for primitive in mesh.primitives() {
                work.poll()?;
                work.complete([1, 0])?;
                let start = storage.targets.len();
                let source_targets = json_primitive(ctx, document, mesh.index(), primitive.index())?.targets.as_deref().unwrap_or(&[]);
                let vertices = mesh_geometry.primitives.as_slice()[primitive.index()].position_count;
                for target_index in 0..width {
                    if target_index.is_multiple_of(PARSER_POLL_INTERVAL) { work.poll()?; }
                    let delta_start = storage.deltas.len();
                    if let Some(target) = source_targets.get(target_index) {
                        for (_, semantic, accessor) in target_accessors(target) {
                            let Some(accessor) = accessor else { continue };
                            let declared_bounds = if semantic == MorphDeltaSemantic::Position {
                                Some(validate_position_bounds(ctx, &document.as_json().accessors[accessor.value()], accessor.value())?)
                            } else { None };
                            let accessor = document.accessors().nth(accessor.value()).ok_or_else(|| gltf_parse_error(ctx, "glTF morph accessor disappeared"))?;
                            let provenance = accessor_provenance(ctx, buffers, accessor.clone())?;
                            let values = safe_gltf(ctx, "glTF morph delta reader", || {
                                gltf::accessor::Iter::<[f32; 3]>::new(accessor, |buffer| buffers.get(buffer.index()).and_then(Option::as_ref).map(BufferData::as_slice))
                            })?.ok_or_else(|| gltf_parse_error(ctx, "glTF morph delta could not be read"))?;
                            let count = vertices.checked_mul(3).ok_or_else(|| ctx.allocation_failure("gltf-morph-value-count"))?;
                            let mut output = try_vec_capacity(ctx, class, count, "gltf-morph-delta-values")?;
                            let mut minimum = [f32::INFINITY; 3];
                            let mut maximum = [f32::NEG_INFINITY; 3];
                            for value in values {
                                if output.len().is_multiple_of(PARSER_POLL_INTERVAL) { work.poll()?; }
                                if output.len() >= count || value.iter().any(|value| !value.is_finite()) {
                                    return Err(gltf_parse_error(ctx, "glTF morph delta must have the declared finite VEC3 values"));
                                }
                                if declared_bounds.is_some() {
                                    for axis in 0..3 {
                                        minimum[axis] = minimum[axis].min(value[axis]);
                                        maximum[axis] = maximum[axis].max(value[axis]);
                                    }
                                }
                                output.try_extend_from_slice_reserved(&value)?;
                                work.complete([3, 3])?;
                            }
                            if output.len() != count { return Err(gltf_parse_error(ctx, "glTF morph delta value count changed after preflight")); }
                            if declared_bounds.is_some_and(|bounds| bounds != (minimum, maximum)) {
                                return Err(gltf_parse_error(ctx, "glTF morph POSITION bounds do not match the delta values"));
                            }
                            storage.deltas.try_push_reserved(MorphDeltaStorage { semantic, values: output, provenance })?;
                            work.complete([1, 0])?;
                        }
                    }
                    storage.targets.try_push_reserved(MorphTargetStorage { deltas: delta_start..storage.deltas.len() })?;
                    work.complete([1, 1])?;
                }
                storage.primitives.try_push_reserved(start..storage.targets.len())?;
            }
        }
        Ok(storage)
    })
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
