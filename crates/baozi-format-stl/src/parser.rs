use crate::detect::{StlKind, detect_bytes};
use crate::{ascii, binary};
use baozi_core::{
    Aabb, BaoziError, Color, Material, Mesh, MeshBinding, MetadataValue, Node, PrimitiveTopology,
    Result, Scene, SceneBuilder, Vec3,
};
use baozi_import::ImportContext;

pub(crate) struct ParsedStl {
    pub storage: &'static str,
    pub meshes: Vec<ParsedMesh>,
    pub material_color: Option<Color>,
}

pub(crate) struct ParsedMesh {
    pub name: Option<String>,
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub colors: Option<Vec<Color>>,
}

pub(crate) fn read_stl(ctx: &mut ImportContext<'_>) -> Result<Scene> {
    let bytes = ctx.read_primary_to_end()?;
    let parsed = match detect_bytes(&bytes) {
        Some(StlKind::Binary { facets }) => binary::parse(ctx, &bytes, facets)?,
        Some(StlKind::Ascii) => ascii::parse(ctx, &bytes)?,
        None => {
            return Err(BaoziError::parse(
                ctx.source().to_string(),
                None,
                "input is not recognized as binary or ASCII STL",
            ));
        }
    };

    scene_from_parsed(ctx, parsed)
}

fn scene_from_parsed(ctx: &ImportContext<'_>, parsed: ParsedStl) -> Result<Scene> {
    if parsed.meshes.is_empty() {
        return Err(BaoziError::parse(
            ctx.source().to_string(),
            None,
            "STL contains no non-empty solids or facets",
        ));
    }
    if parsed.meshes.len() > ctx.limits().max_meshes {
        return Err(BaoziError::LimitExceeded {
            limit: "max_meshes",
        });
    }

    let mut builder = SceneBuilder::new();
    let material = builder.add_material(Material {
        name: Some("STL Default Material".to_owned()),
        base_color: parsed.material_color.unwrap_or(Color::WHITE),
        ..Material::default()
    });

    for parsed_mesh in parsed.meshes {
        let bounds = compute_bounds(&parsed_mesh.positions);
        let mut metadata = baozi_core::MetadataMap::new();
        metadata.insert(
            "stl.storage".to_owned(),
            MetadataValue::String(parsed.storage.to_owned()),
        );
        metadata.insert(
            "stl.source".to_owned(),
            MetadataValue::String(ctx.source().to_string()),
        );
        let mesh = Mesh {
            name: parsed_mesh.name.clone(),
            topology: PrimitiveTopology::Triangles,
            indices: sequential_indices(parsed_mesh.positions.len())?,
            positions: parsed_mesh.positions,
            normals: parsed_mesh.normals,
            colors: parsed_mesh.colors.into_iter().collect(),
            material: Some(material),
            bounds,
            metadata,
            ..Mesh::default()
        };
        let mesh_id = builder.add_mesh(mesh);
        builder.add_child_node(
            builder.root(),
            Node {
                name: parsed_mesh.name,
                mesh_bindings: vec![MeshBinding::new(mesh_id)],
                ..Node::default()
            },
        )?;
    }

    builder.finish()
}

fn sequential_indices(vertex_count: usize) -> Result<Vec<u32>> {
    if vertex_count > u32::MAX as usize {
        return Err(BaoziError::LimitExceeded {
            limit: "max_vertices",
        });
    }
    Ok((0..vertex_count as u32).collect())
}

fn compute_bounds(positions: &[Vec3]) -> Option<Aabb> {
    let first = *positions.first()?;
    let mut min = first;
    let mut max = first;
    for position in positions.iter().copied().skip(1) {
        min.x = min.x.min(position.x);
        min.y = min.y.min(position.y);
        min.z = min.z.min(position.z);
        max.x = max.x.max(position.x);
        max.y = max.y.max(position.y);
        max.z = max.z.max(position.z);
    }
    Some(Aabb { min, max })
}
