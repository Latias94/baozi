use crate::detect::exact_binary_facets;
use crate::error::{StlFailure, StlFailureCode};
use crate::{ascii, binary};
use baozi_core::{
    Aabb, AlphaMode, Color, Mat4, MeshBinding, PrimitiveTopology, SceneDraft, ShadingModel, Vec3,
};
use baozi_import::{
    ImportStorageClass, ImportWorkClass, MaterialBuildConfig, MeshBuildConfig, MeteredString,
    MeteredVec, NodeBuildConfig, ParseFailure, ParseResult, ParseScope, SceneValueParts,
};

pub(crate) struct ParsedStl<'scope> {
    pub meshes: MeteredVec<'scope, ParsedMesh<'scope>>,
    pub material_color: Option<Color>,
}

pub(crate) struct ParsedMesh<'scope> {
    pub name: Option<MeteredString<'scope>>,
    pub positions: MeteredVec<'scope, Vec3>,
    pub normals: MeteredVec<'scope, Vec3>,
    pub colors: Option<MeteredVec<'scope, Color>>,
}

pub(crate) fn read_stl(scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
    let bytes = scope.primary_bytes();
    let parsed = match exact_binary_facets(bytes) {
        Some(facets) => binary::parse(scope, bytes, facets)?,
        None => ascii::parse(scope, bytes)?,
    };
    scene_from_parsed(scope, parsed)
}

fn scene_from_parsed<'scope>(
    scope: &'scope ParseScope<'_>,
    parsed: ParsedStl<'scope>,
) -> ParseResult<SceneDraft> {
    if parsed.meshes.is_empty() {
        return Err(scope.format_failure(StlFailure::content(
            StlFailureCode::EmptyGeometry,
            "document",
            "no-non-empty-solids-or-facets",
        )));
    }

    let mut remaining_topology_cells = 0usize;
    for (index, mesh) in parsed.meshes.as_slice().iter().enumerate() {
        if index % 4096 == 0 {
            scope.poll()?;
        }
        remaining_topology_cells = remaining_topology_cells
            .checked_add(mesh.positions.len())
            .ok_or_else(|| scope.allocation_failure("stl-topology-cell-counter"))?;
    }
    let mut remaining_canonical_string_bytes = canonical_string_bytes(scope, &parsed)?;

    let ParsedStl {
        meshes,
        material_color,
    } = parsed;
    let mut builder = scope.scene_builder()?;
    let material_name_value = "STL Default Material";
    let material_name_bytes = material_name_value.len();
    remaining_canonical_string_bytes = remaining_canonical_string_bytes
        .checked_sub(material_name_bytes)
        .ok_or_else(|| scope.allocation_failure("stl-canonical-string-counter"))?;
    let material = scope.with_work(
        ImportWorkClass::CanonicalStringBytes,
        usize_amount(scope, material_name_bytes, "stl-material-name")?,
        |work| -> ParseResult<_> {
            let material_name = work.run_all(|| {
                metered_string(
                    scope,
                    ImportStorageClass::SceneConstruction,
                    material_name_value,
                    "stl-material-name",
                )
            })?;
            builder.add_prepared_material(
                material_name
                    .try_into_scene_parts()?
                    .into_some()
                    .into_material(MaterialBuildConfig {
                        shading_model: ShadingModel::Unknown,
                        base_color: material_color.unwrap_or(Color::WHITE),
                        metallic: 0.0,
                        roughness: 1.0,
                        emissive: Color::linear_rgba(0.0, 0.0, 0.0, 1.0),
                        alpha_mode: AlphaMode::Opaque,
                        alpha_cutoff: 0.5,
                        double_sided: false,
                    })
                    .finish()?,
            )
        },
    )?;

    meshes.try_for_each_owned(|parsed_mesh| {
        let topology_cells = parsed_mesh.positions.len();
        remaining_topology_cells = remaining_topology_cells
            .checked_sub(topology_cells)
            .ok_or_else(|| scope.allocation_failure("stl-topology-cell-counter"))?;
        let mesh_name_bytes = parsed_mesh.name.as_ref().map_or(0, MeteredString::len);
        let canonical_name_bytes = mesh_name_bytes
            .checked_mul(2)
            .ok_or_else(|| scope.allocation_failure("stl-string-byte-counter"))?;
        remaining_canonical_string_bytes = remaining_canonical_string_bytes
            .checked_sub(canonical_name_bytes)
            .ok_or_else(|| scope.allocation_failure("stl-canonical-string-counter"))?;
        let topology_cells = usize_amount(scope, topology_cells, "stl-topology-cells")?;
        let mesh_name_bytes = usize_amount(scope, mesh_name_bytes, "stl-mesh-name")?;
        let canonical_name_bytes =
            usize_amount(scope, canonical_name_bytes, "stl-canonical-string-bytes")?;
        let attribute_stream_count = 2 + u64::from(parsed_mesh.colors.is_some());
        scope.with_work_group(
            [
                (ImportWorkClass::TopologyCells, topology_cells),
                (ImportWorkClass::CanonicalStringBytes, canonical_name_bytes),
                (ImportWorkClass::Primitives, 1),
                (ImportWorkClass::AttributeStreams, attribute_stream_count),
                (ImportWorkClass::GraphNodes, 1),
                (ImportWorkClass::GraphEdges, 1),
            ],
            |work| -> ParseResult<()> {
                let ParsedMesh {
                    name,
                    positions,
                    normals,
                    colors,
                } = parsed_mesh;
                let bounds = compute_bounds(scope, positions.as_slice())?;
                let indices = work.run([topology_cells, 0, 0, 0, 0, 0], || {
                    sequential_indices(scope, positions.len())
                })?;
                let node_name = name
                    .as_ref()
                    .map(|name| {
                        work.run([0, mesh_name_bytes, 0, 0, 0, 0], || {
                            metered_string(
                                scope,
                                ImportStorageClass::SceneConstruction,
                                name.as_str(),
                                "stl-node-name",
                            )
                        })
                    })
                    .transpose()?;
                let mut color_channels = scope.metered_scene_vec("stl-color-channels");
                if let Some(colors) = colors {
                    color_channels.try_push_parts(colors.try_into_scene_parts()?)?;
                }
                let mesh_id = work.run(
                    [0, mesh_name_bytes, 1, attribute_stream_count, 0, 0],
                    || -> ParseResult<_> {
                        let name = optional_string_parts(scope, name, "stl-mesh-name")?;
                        let mesh = name
                            .into_mesh(MeshBuildConfig {
                                topology: PrimitiveTopology::Triangles,
                                material: Some(material),
                                bounds,
                            })
                            .with_indices(indices.try_into_scene_parts()?)?
                            .with_positions(positions.try_into_scene_parts()?)?
                            .with_normals(normals.try_into_scene_parts()?)?
                            .with_colors(color_channels.try_into_scene_parts()?)?
                            .finish()?;
                        builder.add_prepared_mesh(mesh)
                    },
                )?;
                let node_id = work.run([0, 0, 0, 0, 1, 0], || -> ParseResult<_> {
                    let node = optional_string_parts(scope, node_name, "stl-node-name")?
                        .into_node(NodeBuildConfig {
                            transform: Mat4::IDENTITY,
                            camera: None,
                            light: None,
                        })
                        .finish()?;
                    builder.add_prepared_node(node)
                })?;
                work.run([0, 0, 0, 0, 0, 1], || {
                    builder.add_node_mesh_binding(node_id, MeshBinding::new(mesh_id))
                })
            },
        )
    })?;

    debug_assert_eq!(remaining_topology_cells, 0);
    debug_assert_eq!(remaining_canonical_string_bytes, 0);
    builder.finish_draft()
}

fn sequential_indices<'scope>(
    scope: &'scope ParseScope<'_>,
    vertex_count: usize,
) -> ParseResult<MeteredVec<'scope, u32>> {
    let count =
        u32::try_from(vertex_count).map_err(|_| scope.allocation_failure("stl-index-space"))?;
    let mut indices = scope.metered_vec(ImportStorageClass::SceneConstruction, "stl-indices");
    indices.try_resize(vertex_count, 0)?;
    let mut start = 0usize;
    while start < vertex_count {
        scope.poll()?;
        let end = start.saturating_add(4096).min(vertex_count);
        for (offset, index) in indices.as_mut_slice()[start..end].iter_mut().enumerate() {
            *index = u32::try_from(start + offset)
                .map_err(|_| scope.allocation_failure("stl-index-space"))?;
        }
        start = end;
    }
    debug_assert_eq!(indices.len(), usize::try_from(count).unwrap_or(usize::MAX));
    Ok(indices)
}

fn canonical_string_bytes(scope: &ParseScope<'_>, parsed: &ParsedStl<'_>) -> ParseResult<usize> {
    let mut bytes = "STL Default Material".len();
    for (index, mesh) in parsed.meshes.as_slice().iter().enumerate() {
        if index % 4096 == 0 {
            scope.poll()?;
        }
        if let Some(name) = &mesh.name {
            checked_add(scope, &mut bytes, name.len(), "stl-string-byte-counter")?;
            checked_add(scope, &mut bytes, name.len(), "stl-string-byte-counter")?;
        }
    }
    Ok(bytes)
}

fn checked_add(
    scope: &ParseScope<'_>,
    total: &mut usize,
    value: usize,
    resource: &'static str,
) -> ParseResult<()> {
    *total = total
        .checked_add(value)
        .ok_or_else(|| scope.allocation_failure(resource))?;
    Ok(())
}

fn metered_string<'scope>(
    scope: &'scope ParseScope<'_>,
    class: ImportStorageClass,
    value: &str,
    resource: &'static str,
) -> ParseResult<MeteredString<'scope>> {
    let mut output = scope.metered_string(class, resource);
    output.try_push_str(value)?;
    Ok(output)
}

fn optional_string_parts<'scope>(
    scope: &'scope ParseScope<'_>,
    value: Option<MeteredString<'scope>>,
    resource: &'static str,
) -> ParseResult<SceneValueParts<'scope, Option<String>>> {
    match value {
        Some(value) => Ok(value.try_into_scene_parts()?.into_some()),
        None => Ok(scope
            .metered_string(ImportStorageClass::SceneConstruction, resource)
            .try_into_scene_parts()?
            .discard_into_none()),
    }
}

fn usize_amount(
    scope: &ParseScope<'_>,
    value: usize,
    resource: &'static str,
) -> Result<u64, ParseFailure> {
    u64::try_from(value).map_err(|_| scope.allocation_failure(resource))
}

fn compute_bounds(scope: &ParseScope<'_>, positions: &[Vec3]) -> ParseResult<Option<Aabb>> {
    let Some(first) = positions.first().copied() else {
        return Ok(None);
    };
    let mut min = first;
    let mut max = first;
    for (index, position) in positions.iter().copied().enumerate().skip(1) {
        if index % 4096 == 0 {
            scope.poll()?;
        }
        min.x = min.x.min(position.x);
        min.y = min.y.min(position.y);
        min.z = min.z.min(position.z);
        max.x = max.x.max(position.x);
        max.y = max.y.max(position.y);
        max.z = max.z.max(position.z);
    }
    Ok(Some(Aabb { min, max }))
}
