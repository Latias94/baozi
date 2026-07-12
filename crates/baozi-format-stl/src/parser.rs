use crate::allocation::LiveStorage;
use crate::detect::exact_binary_facets;
use crate::error::{StlFailure, StlFailureCode};
use crate::{ascii, binary};
use baozi_core::{
    Aabb, Color, Material, Mesh, MeshBinding, Node, PrimitiveTopology, SceneDraft, Vec3,
};
use baozi_import::{ImportStorageClass, ImportWorkClass, ParseFailure, ParseResult, ParseScope};

pub(crate) struct ParsedStl<'scope> {
    pub meshes: Vec<ParsedMesh<'scope>>,
    pub material_color: Option<Color>,
    pub reservations: LiveStorage<'scope>,
}

pub(crate) struct ParsedMesh<'scope> {
    pub name: Option<String>,
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub colors: Option<Vec<Color>>,
    pub reservations: LiveStorage<'scope>,
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
    for (index, mesh) in parsed.meshes.iter().enumerate() {
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
        reservations: mesh_list_reservations,
    } = parsed;
    let mut builder = scope.scene_builder()?;
    let mut material_reservations = LiveStorage::new(ImportStorageClass::SceneConstruction);
    let material_name = owned_string(
        scope,
        &mut material_reservations,
        "STL Default Material",
        "stl-material-name",
    )?;
    material_reservations.adopt(&mut builder)?;
    let material = builder.add_material(Material {
        name: Some(material_name),
        base_color: material_color.unwrap_or(Color::WHITE),
        ..Material::default()
    })?;

    {
        let _mesh_list_reservations = mesh_list_reservations;
        for parsed_mesh in meshes {
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
                mut reservations,
            } = parsed_mesh;
            let bounds = compute_bounds(scope, &positions)?;
            let indices = sequential_indices(scope, &mut reservations, positions.len())?;
            let node_name = name
                .as_deref()
                .map(|name| owned_string(scope, &mut reservations, name, "stl-node-name"))
                .transpose()?;
            let colors = match colors {
                Some(channel) => {
                    let mut channels = Vec::new();
                    reservations.reserve_vec(scope, &mut channels, 1, "stl-color-channels")?;
                    channels.push(channel);
                    channels
                }
                None => Vec::new(),
            };
            let mut mesh_bindings = Vec::new();
            reservations.reserve_vec(scope, &mut mesh_bindings, 1, "stl-mesh-bindings")?;
            let mesh = Mesh {
                name,
                topology: PrimitiveTopology::Triangles,
                indices,
                positions,
                normals,
                colors,
                material: Some(material),
                bounds,
                ..Mesh::default()
            };
            reservations.adopt(&mut builder)?;
            let mesh_id = builder.add_mesh(mesh)?;
            mesh_bindings.push(MeshBinding::new(mesh_id));
            builder.add_child_node(
                builder.root(),
                Node {
                    name: node_name,
                    mesh_bindings,
                    ..Node::default()
                },
            )?;
            graph_edge.complete()?;
            graph_node.complete()?;
            attribute_streams.complete()?;
            primitive.complete()?;
        }
    }

    strings.complete()?;
    topology.complete()?;
    builder.finish_draft()
}

fn sequential_indices<'scope>(
    scope: &'scope ParseScope<'_>,
    reservations: &mut LiveStorage<'scope>,
    vertex_count: usize,
) -> ParseResult<Vec<u32>> {
    let count =
        u32::try_from(vertex_count).map_err(|_| scope.allocation_failure("stl-index-space"))?;
    let mut indices = Vec::new();
    reservations.reserve_vec(scope, &mut indices, vertex_count, "stl-indices")?;
    let mut start = 0u32;
    while start < count {
        scope.poll()?;
        let end = start.saturating_add(4096).min(count);
        indices.extend(start..end);
        start = end;
    }
    Ok(indices)
}

fn canonical_string_bytes(scope: &ParseScope<'_>, parsed: &ParsedStl<'_>) -> ParseResult<usize> {
    let mut bytes = "STL Default Material".len();
    for (index, mesh) in parsed.meshes.iter().enumerate() {
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

fn owned_string<'scope>(
    scope: &'scope ParseScope<'_>,
    reservations: &mut LiveStorage<'scope>,
    value: &str,
    resource: &'static str,
) -> ParseResult<String> {
    reservations.copy_string(scope, value, resource)
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
