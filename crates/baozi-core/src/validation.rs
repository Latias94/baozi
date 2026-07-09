use crate::{
    BaoziError, Color, Material, Mesh, PrimitiveTopology, Result, Scene, TextureId, Vec2, Vec3,
    Vec4,
};

pub fn validate_scene(scene: &Scene) -> Result<()> {
    validate_root(scene)?;
    validate_materials(scene)?;
    validate_meshes(scene)?;
    validate_nodes(scene)?;
    validate_space(scene)?;
    Ok(())
}

fn validate_root(scene: &Scene) -> Result<()> {
    let root = scene.root.index();
    let Some(root_node) = scene.nodes.get(root) else {
        return invalid("root node is out of range");
    };
    if root_node.parent.is_some() {
        return invalid("root node must not have a parent");
    }
    Ok(())
}

fn validate_nodes(scene: &Scene) -> Result<()> {
    let mut state = vec![VisitState::Unvisited; scene.nodes.len()];

    for (index, node) in scene.nodes.iter().enumerate() {
        if !node.transform.is_finite() {
            return invalid(format!(
                "node {index} transform contains a non-finite value"
            ));
        }

        if let Some(parent) = node.parent
            && parent.index() >= scene.nodes.len()
        {
            return invalid(format!("node {index} parent is out of range"));
        }

        for child in &node.children {
            let Some(child_node) = scene.nodes.get(child.index()) else {
                return invalid(format!("node {index} child is out of range"));
            };
            if child_node.parent != Some(crate::NodeId::new(index as u32)) {
                return invalid(format!(
                    "node {} parent does not point back to node {index}",
                    child.0
                ));
            }
        }

        for mesh in &node.meshes {
            if mesh.index() >= scene.meshes.len() {
                return invalid(format!("node {index} mesh reference is out of range"));
            }
        }
    }

    visit_node(scene, scene.root.index(), &mut state)?;
    for (index, state) in state.iter().enumerate() {
        if *state == VisitState::Unvisited {
            return invalid(format!("node {index} is not reachable from the root"));
        }
    }

    Ok(())
}

fn validate_materials(scene: &Scene) -> Result<()> {
    for (index, material) in scene.materials.iter().enumerate() {
        validate_material(index, material, scene.textures.len())?;
    }
    Ok(())
}

fn validate_material(index: usize, material: &Material, texture_count: usize) -> Result<()> {
    if !color_is_finite(material.base_color)
        || !color_is_finite(material.emissive)
        || !material.metallic.is_finite()
        || !material.roughness.is_finite()
        || !material.alpha_cutoff.is_finite()
    {
        return invalid(format!("material {index} contains a non-finite value"));
    }

    for slot in &material.texture_slots {
        validate_texture_id(index, slot.texture, texture_count)?;
        if !slot.scale.is_finite() {
            return invalid(format!("material {index} texture scale is non-finite"));
        }
    }

    Ok(())
}

fn validate_texture_id(
    material_index: usize,
    texture: TextureId,
    texture_count: usize,
) -> Result<()> {
    if texture.index() >= texture_count {
        return invalid(format!(
            "material {material_index} texture reference is out of range"
        ));
    }
    Ok(())
}

fn validate_meshes(scene: &Scene) -> Result<()> {
    for (index, mesh) in scene.meshes.iter().enumerate() {
        validate_mesh(index, mesh, scene.materials.len())?;
    }
    Ok(())
}

fn validate_mesh(index: usize, mesh: &Mesh, material_count: usize) -> Result<()> {
    let vertex_count = mesh.positions.len();
    if vertex_count == 0 {
        return invalid(format!("mesh {index} is empty: no positions"));
    }

    if let Some(material) = mesh.material
        && material.index() >= material_count
    {
        return invalid(format!("mesh {index} material reference is out of range"));
    }

    validate_vec3_channel(index, "positions", &mesh.positions, Some(vertex_count))?;
    validate_vec3_channel(
        index,
        "normals",
        &mesh.normals,
        optional_len(vertex_count, &mesh.normals),
    )?;
    validate_vec4_channel(
        index,
        "tangents",
        &mesh.tangents,
        optional_len(vertex_count, &mesh.tangents),
    )?;
    for (channel, texcoords) in mesh.texcoords.iter().enumerate() {
        validate_vec2_channel(
            index,
            &format!("texcoords[{channel}]"),
            texcoords,
            Some(vertex_count),
        )?;
    }
    for (channel, colors) in mesh.colors.iter().enumerate() {
        validate_color_channel(
            index,
            &format!("colors[{channel}]"),
            colors,
            Some(vertex_count),
        )?;
    }

    for index_value in &mesh.indices {
        if *index_value as usize >= vertex_count {
            return invalid(format!("mesh {index} index {index_value} is out of range"));
        }
    }

    validate_topology(
        index,
        mesh.topology,
        mesh.element_count(),
        &mesh.face_vertex_counts,
    )?;

    if let Some(bounds) = mesh.bounds {
        if !bounds.min.is_finite() || !bounds.max.is_finite() {
            return invalid(format!("mesh {index} bounds contain a non-finite value"));
        }
        if bounds.min.x > bounds.max.x || bounds.min.y > bounds.max.y || bounds.min.z > bounds.max.z
        {
            return invalid(format!("mesh {index} bounds min exceeds max"));
        }
    }

    Ok(())
}

fn optional_len<T>(vertex_count: usize, values: &[T]) -> Option<usize> {
    (!values.is_empty()).then_some(vertex_count)
}

fn validate_vec2_channel(
    mesh_index: usize,
    name: &str,
    values: &[Vec2],
    expected_len: Option<usize>,
) -> Result<()> {
    validate_channel_len(mesh_index, name, values.len(), expected_len)?;
    if values.iter().any(|value| !value.is_finite()) {
        return invalid(format!(
            "mesh {mesh_index} {name} contains a non-finite value"
        ));
    }
    Ok(())
}

fn validate_vec3_channel(
    mesh_index: usize,
    name: &str,
    values: &[Vec3],
    expected_len: Option<usize>,
) -> Result<()> {
    validate_channel_len(mesh_index, name, values.len(), expected_len)?;
    if values.iter().any(|value| !value.is_finite()) {
        return invalid(format!(
            "mesh {mesh_index} {name} contains a non-finite value"
        ));
    }
    Ok(())
}

fn validate_vec4_channel(
    mesh_index: usize,
    name: &str,
    values: &[Vec4],
    expected_len: Option<usize>,
) -> Result<()> {
    validate_channel_len(mesh_index, name, values.len(), expected_len)?;
    if values.iter().any(|value| !value.is_finite()) {
        return invalid(format!(
            "mesh {mesh_index} {name} contains a non-finite value"
        ));
    }
    Ok(())
}

fn validate_color_channel(
    mesh_index: usize,
    name: &str,
    values: &[Color],
    expected_len: Option<usize>,
) -> Result<()> {
    validate_channel_len(mesh_index, name, values.len(), expected_len)?;
    if values.iter().any(|value| !color_is_finite(*value)) {
        return invalid(format!(
            "mesh {mesh_index} {name} contains a non-finite value"
        ));
    }
    Ok(())
}

fn validate_channel_len(
    mesh_index: usize,
    name: &str,
    actual_len: usize,
    expected_len: Option<usize>,
) -> Result<()> {
    if let Some(expected_len) = expected_len
        && actual_len != expected_len
    {
        return invalid(format!(
            "mesh {mesh_index} {name} length {actual_len} does not match positions length {expected_len}"
        ));
    }
    Ok(())
}

fn validate_topology(
    mesh_index: usize,
    topology: PrimitiveTopology,
    element_count: usize,
    face_vertex_counts: &[u32],
) -> Result<()> {
    match topology {
        PrimitiveTopology::Points | PrimitiveTopology::Lines | PrimitiveTopology::Triangles
            if !face_vertex_counts.is_empty() =>
        {
            invalid(format!(
                "mesh {mesh_index} face counts are only valid for polygon topology"
            ))
        }
        PrimitiveTopology::Points => Ok(()),
        PrimitiveTopology::Lines if element_count.is_multiple_of(2) => Ok(()),
        PrimitiveTopology::Triangles if element_count.is_multiple_of(3) => Ok(()),
        PrimitiveTopology::Lines => invalid(format!(
            "mesh {mesh_index} line topology element count is not divisible by 2"
        )),
        PrimitiveTopology::Triangles => invalid(format!(
            "mesh {mesh_index} triangle topology element count is not divisible by 3"
        )),
        PrimitiveTopology::Polygons => {
            validate_polygon_faces(mesh_index, element_count, face_vertex_counts)
        }
    }
}

fn validate_polygon_faces(
    mesh_index: usize,
    element_count: usize,
    face_vertex_counts: &[u32],
) -> Result<()> {
    if face_vertex_counts.is_empty() {
        return invalid(format!(
            "mesh {mesh_index} polygon topology lacks face range data"
        ));
    }

    let mut total = 0usize;
    for (face_index, count) in face_vertex_counts.iter().copied().enumerate() {
        if count < 3 {
            return invalid(format!(
                "mesh {mesh_index} polygon face {face_index} has fewer than 3 vertices"
            ));
        }
        total = total
            .checked_add(count as usize)
            .ok_or_else(|| BaoziError::InvalidScene {
                message: format!("mesh {mesh_index} polygon face counts overflow"),
            })?;
    }

    if total != element_count {
        return invalid(format!(
            "mesh {mesh_index} polygon face counts total {total} does not match element count {element_count}"
        ));
    }

    Ok(())
}

fn validate_space(scene: &Scene) -> Result<()> {
    if let Some(scale) = scene.space.unit_scale_to_meters
        && (!scale.is_finite() || scale <= 0.0)
    {
        return invalid("scene unit scale must be finite and positive");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Unvisited,
    Visiting,
    Visited,
}

fn visit_node(scene: &Scene, index: usize, state: &mut [VisitState]) -> Result<()> {
    match state[index] {
        VisitState::Visiting => {
            return invalid(format!("node hierarchy contains a cycle at node {index}"));
        }
        VisitState::Visited => return Ok(()),
        VisitState::Unvisited => {}
    }

    state[index] = VisitState::Visiting;
    for child in &scene.nodes[index].children {
        visit_node(scene, child.index(), state)?;
    }
    state[index] = VisitState::Visited;
    Ok(())
}

fn color_is_finite(color: Color) -> bool {
    color.r.is_finite() && color.g.is_finite() && color.b.is_finite() && color.a.is_finite()
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(BaoziError::InvalidScene {
        message: message.into(),
    })
}
