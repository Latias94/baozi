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
    let decode = scope.begin_work(
        ImportWorkClass::DecodeWork,
        usize_amount(scope, bytes.len(), "stl-decode-work")?,
    )?;
    let parsed = match exact_binary_facets(bytes) {
        Some(facets) => binary::parse(scope, bytes, facets)?,
        None => ascii::parse(scope, bytes)?,
    };
    decode.complete()?;
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

    let mut topology_cells = 0usize;
    for (index, mesh) in parsed.meshes.as_slice().iter().enumerate() {
        if index % 4096 == 0 {
            scope.poll()?;
        }
        topology_cells = topology_cells
            .checked_add(mesh.positions.len())
            .ok_or_else(|| scope.allocation_failure("stl-topology-cell-counter"))?;
    }
    let topology = scope.begin_work(
        ImportWorkClass::TopologyCells,
        usize_amount(scope, topology_cells, "stl-topology-cells")?,
    )?;
    let canonical_strings = canonical_string_bytes(scope, &parsed)?;
    let strings = scope.begin_work(
        ImportWorkClass::CanonicalStringBytes,
        usize_amount(scope, canonical_strings, "stl-canonical-string-bytes")?,
    )?;

    let ParsedStl {
        meshes,
        material_color,
    } = parsed;
    let mut builder = scope.scene_builder()?;
    let material_name = metered_string(
        scope,
        ImportStorageClass::SceneConstruction,
        "STL Default Material",
        "stl-material-name",
    )?;
    let material = builder.add_prepared_material(
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
    )?;

    meshes.try_for_each_owned(|parsed_mesh| {
        let primitive = scope.begin_work(ImportWorkClass::Primitives, 1)?;
        let attribute_stream_count = 2 + u64::from(parsed_mesh.colors.is_some());
        let attribute_streams =
            scope.begin_work(ImportWorkClass::AttributeStreams, attribute_stream_count)?;
        let graph_node = scope.begin_work(ImportWorkClass::GraphNodes, 1)?;
        let graph_edge = scope.begin_work(ImportWorkClass::GraphEdges, 1)?;
        let ParsedMesh {
            name,
            positions,
            normals,
            colors,
        } = parsed_mesh;
        let bounds = compute_bounds(scope, positions.as_slice())?;
        let indices = sequential_indices(scope, positions.len())?;
        let node_name = name
            .as_ref()
            .map(|name| {
                metered_string(
                    scope,
                    ImportStorageClass::SceneConstruction,
                    name.as_str(),
                    "stl-node-name",
                )
            })
            .transpose()?;
        let mut color_channels = scope.metered_scene_vec("stl-color-channels");
        if let Some(colors) = colors {
            color_channels.try_push_parts(colors.try_into_scene_parts()?)?;
        }
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
        let mesh_id = builder.add_prepared_mesh(mesh)?;
        let node = optional_string_parts(scope, node_name, "stl-node-name")?
            .into_node(NodeBuildConfig {
                transform: Mat4::IDENTITY,
                camera: None,
                light: None,
            })
            .finish()?;
        let node_id = builder.add_prepared_node(node)?;
        builder.add_node_mesh_binding(node_id, MeshBinding::new(mesh_id))?;
        graph_edge.complete()?;
        graph_node.complete()?;
        attribute_streams.complete()?;
        primitive.complete()
    })?;

    strings.complete()?;
    topology.complete()?;
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
