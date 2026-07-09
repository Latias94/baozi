use crate::{
    Animation, AnimationValues, BaoziError, Camera, CameraProjection, Color, Light, Material,
    MaterialProperty, Mesh, PrimitiveTopology, Result, Scene, Skin, Texture, TextureId, Vec2, Vec3,
    Vec4, VertexAttributeData,
};

pub fn validate_scene(scene: &Scene) -> Result<()> {
    validate_root(scene)?;
    validate_materials(scene)?;
    validate_textures(scene)?;
    validate_nodes(scene)?;
    validate_skins(scene)?;
    validate_meshes(scene)?;
    validate_cameras(scene)?;
    validate_lights(scene)?;
    validate_animations(scene)?;
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
                    child.as_u32()
                ));
            }
        }

        for binding in &node.mesh_bindings {
            if binding.mesh.index() >= scene.meshes.len() {
                return invalid(format!("node {index} mesh reference is out of range"));
            }
            if let Some(skin) = binding.skin
                && skin.index() >= scene.skins.len()
            {
                return invalid(format!("node {index} skin reference is out of range"));
            }
        }

        if let Some(camera) = node.camera
            && camera.index() >= scene.cameras.len()
        {
            return invalid(format!("node {index} camera reference is out of range"));
        }
        if let Some(light) = node.light
            && light.index() >= scene.lights.len()
        {
            return invalid(format!("node {index} light reference is out of range"));
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
        if !slot.scale.is_finite()
            || !slot.transform.offset.is_finite()
            || !slot.transform.rotation_radians.is_finite()
            || !slot.transform.scale.is_finite()
        {
            return invalid(format!(
                "material {index} texture slot contains a non-finite value"
            ));
        }
    }

    for (key, property) in &material.properties {
        validate_namespaced_key("material property", key)?;
        validate_material_property(index, key, property, texture_count)?;
    }

    Ok(())
}

fn validate_material_property(
    material_index: usize,
    key: &str,
    property: &MaterialProperty,
    texture_count: usize,
) -> Result<()> {
    match property {
        MaterialProperty::Bool(_) | MaterialProperty::I64(_) | MaterialProperty::String(_) => {
            Ok(())
        }
        MaterialProperty::F64(value) if value.is_finite() => Ok(()),
        MaterialProperty::F64(_) => invalid(format!(
            "material {material_index} property {key} contains a non-finite value"
        )),
        MaterialProperty::Color(value) if color_is_finite(*value) => Ok(()),
        MaterialProperty::Color(_)
        | MaterialProperty::Vec2(_)
        | MaterialProperty::Vec3(_)
        | MaterialProperty::Vec4(_) => {
            let finite = match property {
                MaterialProperty::Color(value) => color_is_finite(*value),
                MaterialProperty::Vec2(value) => value.is_finite(),
                MaterialProperty::Vec3(value) => value.is_finite(),
                MaterialProperty::Vec4(value) => value.is_finite(),
                _ => true,
            };
            if finite {
                Ok(())
            } else {
                invalid(format!(
                    "material {material_index} property {key} contains a non-finite value"
                ))
            }
        }
        MaterialProperty::Texture(texture) => {
            validate_texture_id(material_index, *texture, texture_count)
        }
    }
}

fn validate_textures(scene: &Scene) -> Result<()> {
    for (index, texture) in scene.textures.iter().enumerate() {
        validate_texture(index, texture)?;
    }
    Ok(())
}

fn validate_texture(index: usize, texture: &Texture) -> Result<()> {
    if let crate::TextureSource::External { uri } = &texture.source
        && uri.is_empty()
    {
        return invalid(format!("texture {index} external uri is empty"));
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
    let bound_skins = mesh_bound_skins(scene);
    for (index, mesh) in scene.meshes.iter().enumerate() {
        validate_mesh(
            index,
            mesh,
            scene.materials.len(),
            &scene.skins,
            &bound_skins[index],
        )?;
    }
    Ok(())
}

fn mesh_bound_skins(scene: &Scene) -> Vec<Vec<crate::SkinId>> {
    let mut bound_skins = vec![Vec::new(); scene.meshes.len()];
    for node in &scene.nodes {
        for binding in &node.mesh_bindings {
            if binding.mesh.index() < bound_skins.len()
                && let Some(skin) = binding.skin
            {
                bound_skins[binding.mesh.index()].push(skin);
            }
        }
    }
    bound_skins
}

fn validate_mesh(
    index: usize,
    mesh: &Mesh,
    material_count: usize,
    skins: &[Skin],
    bound_skin_ids: &[crate::SkinId],
) -> Result<()> {
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
    validate_joint_channels(index, vertex_count, mesh, skins, bound_skin_ids)?;
    for (target_index, target) in mesh.morph_targets.iter().enumerate() {
        validate_vec3_channel(
            index,
            &format!("morph_targets[{target_index}].positions"),
            &target.positions,
            optional_len(vertex_count, &target.positions),
        )?;
        validate_vec3_channel(
            index,
            &format!("morph_targets[{target_index}].normals"),
            &target.normals,
            optional_len(vertex_count, &target.normals),
        )?;
        validate_vec4_channel(
            index,
            &format!("morph_targets[{target_index}].tangents"),
            &target.tangents,
            optional_len(vertex_count, &target.tangents),
        )?;
    }
    for (attribute_index, attribute) in mesh.custom_attributes.iter().enumerate() {
        validate_namespaced_key("custom vertex attribute", &attribute.name)?;
        validate_vertex_attribute(index, attribute_index, vertex_count, &attribute.data)?;
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

fn validate_joint_channels(
    mesh_index: usize,
    vertex_count: usize,
    mesh: &Mesh,
    skins: &[Skin],
    bound_skin_ids: &[crate::SkinId],
) -> Result<()> {
    if mesh.joint_indices.is_empty() && mesh.joint_weights.is_empty() {
        return Ok(());
    }
    if mesh.joint_indices.len() != vertex_count || mesh.joint_weights.len() != vertex_count {
        return invalid(format!(
            "mesh {mesh_index} joint indices and weights must both match positions length"
        ));
    }
    if bound_skin_ids.is_empty() {
        return invalid(format!("mesh {mesh_index} joint channels require a skin"));
    }
    let mut min_joint_count = usize::MAX;
    for skin in bound_skin_ids {
        let Some(skin) = skins.get(skin.index()) else {
            return invalid(format!("mesh {mesh_index} skin reference is out of range"));
        };
        min_joint_count = min_joint_count.min(skin.joints.len());
    }
    for (vertex, joints) in mesh.joint_indices.iter().enumerate() {
        if let Some(joint) = joints
            .iter()
            .copied()
            .find(|joint| *joint as usize >= min_joint_count)
        {
            return invalid(format!(
                "mesh {mesh_index} joint index {joint} for vertex {vertex} is out of range for a bound skin"
            ));
        }
    }
    for (vertex, weights) in mesh.joint_weights.iter().enumerate() {
        if weights.iter().any(|weight| !weight.is_finite()) {
            return invalid(format!(
                "mesh {mesh_index} joint weights for vertex {vertex} contain a non-finite value"
            ));
        }
    }
    Ok(())
}

fn validate_vertex_attribute(
    mesh_index: usize,
    attribute_index: usize,
    vertex_count: usize,
    data: &VertexAttributeData,
) -> Result<()> {
    validate_channel_len(
        mesh_index,
        &format!("custom_attributes[{attribute_index}]"),
        data.len(),
        Some(vertex_count),
    )?;
    match data {
        VertexAttributeData::F32(values) if values.iter().any(|value| !value.is_finite()) => {
            invalid(format!(
                "mesh {mesh_index} custom attribute {attribute_index} contains a non-finite value"
            ))
        }
        VertexAttributeData::Vec2(values) if values.iter().any(|value| !value.is_finite()) => {
            invalid(format!(
                "mesh {mesh_index} custom attribute {attribute_index} contains a non-finite value"
            ))
        }
        VertexAttributeData::Vec3(values) if values.iter().any(|value| !value.is_finite()) => {
            invalid(format!(
                "mesh {mesh_index} custom attribute {attribute_index} contains a non-finite value"
            ))
        }
        VertexAttributeData::Vec4(values) if values.iter().any(|value| !value.is_finite()) => {
            invalid(format!(
                "mesh {mesh_index} custom attribute {attribute_index} contains a non-finite value"
            ))
        }
        _ => Ok(()),
    }
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

fn validate_skins(scene: &Scene) -> Result<()> {
    for (index, skin) in scene.skins.iter().enumerate() {
        validate_skin(index, skin, scene.nodes.len())?;
    }
    Ok(())
}

fn validate_skin(index: usize, skin: &Skin, node_count: usize) -> Result<()> {
    if let Some(root) = skin.skeleton_root
        && root.index() >= node_count
    {
        return invalid(format!("skin {index} skeleton root is out of range"));
    }
    for joint in &skin.joints {
        if joint.index() >= node_count {
            return invalid(format!("skin {index} joint reference is out of range"));
        }
    }
    if !skin.inverse_bind_matrices.is_empty()
        && skin.inverse_bind_matrices.len() != skin.joints.len()
    {
        return invalid(format!(
            "skin {index} inverse bind matrix count does not match joint count"
        ));
    }
    if skin
        .inverse_bind_matrices
        .iter()
        .any(|matrix| !matrix.is_finite())
    {
        return invalid(format!(
            "skin {index} inverse bind matrices contain a non-finite value"
        ));
    }
    Ok(())
}

fn validate_cameras(scene: &Scene) -> Result<()> {
    for (index, camera) in scene.cameras.iter().enumerate() {
        validate_camera(index, camera)?;
    }
    Ok(())
}

fn validate_camera(index: usize, camera: &Camera) -> Result<()> {
    match camera.projection {
        CameraProjection::Perspective {
            yfov_radians,
            aspect_ratio,
            znear,
            zfar,
        } => {
            if !yfov_radians.is_finite() || yfov_radians <= 0.0 {
                return invalid(format!("camera {index} perspective yfov is invalid"));
            }
            if let Some(aspect_ratio) = aspect_ratio
                && (!aspect_ratio.is_finite() || aspect_ratio <= 0.0)
            {
                return invalid(format!(
                    "camera {index} perspective aspect ratio is invalid"
                ));
            }
            validate_positive_depth(index, "znear", znear)?;
            if let Some(zfar) = zfar {
                validate_positive_depth(index, "zfar", zfar)?;
                if zfar <= znear {
                    return invalid(format!("camera {index} zfar must exceed znear"));
                }
            }
        }
        CameraProjection::Orthographic {
            xmag,
            ymag,
            znear,
            zfar,
        } => {
            if !xmag.is_finite() || xmag <= 0.0 || !ymag.is_finite() || ymag <= 0.0 {
                return invalid(format!(
                    "camera {index} orthographic magnification is invalid"
                ));
            }
            validate_positive_depth(index, "znear", znear)?;
            validate_positive_depth(index, "zfar", zfar)?;
            if zfar <= znear {
                return invalid(format!("camera {index} zfar must exceed znear"));
            }
        }
        CameraProjection::Unknown => {}
    }
    Ok(())
}

fn validate_positive_depth(camera_index: usize, name: &str, value: f32) -> Result<()> {
    if !value.is_finite() || value <= 0.0 {
        return invalid(format!("camera {camera_index} {name} is invalid"));
    }
    Ok(())
}

fn validate_lights(scene: &Scene) -> Result<()> {
    for (index, light) in scene.lights.iter().enumerate() {
        validate_light(index, light)?;
    }
    Ok(())
}

fn validate_light(index: usize, light: &Light) -> Result<()> {
    if !color_is_finite(light.color) || !light.intensity.is_finite() {
        return invalid(format!("light {index} contains a non-finite value"));
    }
    if light.intensity < 0.0 {
        return invalid(format!("light {index} intensity is negative"));
    }
    if let Some(range) = light.range
        && (!range.is_finite() || range < 0.0)
    {
        return invalid(format!("light {index} range is invalid"));
    }
    if let Some(angle) = light.inner_cone_angle
        && (!angle.is_finite() || angle < 0.0)
    {
        return invalid(format!("light {index} inner cone angle is invalid"));
    }
    if let Some(angle) = light.outer_cone_angle
        && (!angle.is_finite() || angle < 0.0)
    {
        return invalid(format!("light {index} outer cone angle is invalid"));
    }
    if let (Some(inner), Some(outer)) = (light.inner_cone_angle, light.outer_cone_angle)
        && inner > outer
    {
        return invalid(format!(
            "light {index} inner cone angle exceeds outer cone angle"
        ));
    }
    Ok(())
}

fn validate_animations(scene: &Scene) -> Result<()> {
    for (index, animation) in scene.animations.iter().enumerate() {
        validate_animation(index, animation, scene.nodes.len())?;
    }
    Ok(())
}

fn validate_animation(
    animation_index: usize,
    animation: &Animation,
    node_count: usize,
) -> Result<()> {
    for (channel_index, channel) in animation.channels.iter().enumerate() {
        if channel.target.node.index() >= node_count {
            return invalid(format!(
                "animation {animation_index} channel {channel_index} target node is out of range"
            ));
        }
        if channel.times_seconds.is_empty() {
            return invalid(format!(
                "animation {animation_index} channel {channel_index} has no keyframe times"
            ));
        }
        if channel.times_seconds.iter().any(|time| !time.is_finite()) {
            return invalid(format!(
                "animation {animation_index} channel {channel_index} contains a non-finite time"
            ));
        }
        for window in channel.times_seconds.windows(2) {
            if window[0] > window[1] {
                return invalid(format!(
                    "animation {animation_index} channel {channel_index} times are not sorted"
                ));
            }
        }
        validate_animation_values(
            animation_index,
            channel_index,
            channel.target.property,
            channel.interpolation,
            channel.times_seconds.len(),
            &channel.values,
        )?;
    }
    Ok(())
}

fn validate_animation_values(
    animation_index: usize,
    channel_index: usize,
    property: crate::AnimationProperty,
    interpolation: crate::AnimationInterpolation,
    keyframes: usize,
    values: &AnimationValues,
) -> Result<()> {
    if !animation_values_match_property(property, values) {
        return invalid(format!(
            "animation {animation_index} channel {channel_index} value kind does not match target property"
        ));
    }

    let sample_count = match values {
        AnimationValues::Translations(values) => {
            if values.iter().any(|value| !value.is_finite()) {
                return invalid(format!(
                    "animation {animation_index} channel {channel_index} translations contain a non-finite value"
                ));
            }
            values.len()
        }
        AnimationValues::Rotations(values) => {
            if values.iter().any(|value| !value.is_finite()) {
                return invalid(format!(
                    "animation {animation_index} channel {channel_index} rotations contain a non-finite value"
                ));
            }
            values.len()
        }
        AnimationValues::Scales(values) => {
            if values.iter().any(|value| !value.is_finite()) {
                return invalid(format!(
                    "animation {animation_index} channel {channel_index} scales contain a non-finite value"
                ));
            }
            values.len()
        }
        AnimationValues::MorphWeights {
            values,
            weights_per_keyframe,
        } => {
            if *weights_per_keyframe == 0 {
                return invalid(format!(
                    "animation {animation_index} channel {channel_index} has zero morph weights per keyframe"
                ));
            }
            if values.iter().any(|value| !value.is_finite()) {
                return invalid(format!(
                    "animation {animation_index} channel {channel_index} morph weights contain a non-finite value"
                ));
            }
            if values.len() % weights_per_keyframe != 0 {
                return invalid(format!(
                    "animation {animation_index} channel {channel_index} morph weight sample count is invalid"
                ));
            }
            values.len() / weights_per_keyframe
        }
    };

    let expected = match interpolation {
        crate::AnimationInterpolation::CubicSpline => keyframes.saturating_mul(3),
        crate::AnimationInterpolation::Step
        | crate::AnimationInterpolation::Linear
        | crate::AnimationInterpolation::Unknown => keyframes,
    };
    if sample_count != expected {
        return invalid(format!(
            "animation {animation_index} channel {channel_index} value count does not match keyframe times"
        ));
    }
    Ok(())
}

fn animation_values_match_property(
    property: crate::AnimationProperty,
    values: &AnimationValues,
) -> bool {
    matches!(
        (property, values),
        (
            crate::AnimationProperty::Translation,
            AnimationValues::Translations(_)
        ) | (
            crate::AnimationProperty::Rotation,
            AnimationValues::Rotations(_)
        ) | (crate::AnimationProperty::Scale, AnimationValues::Scales(_))
            | (
                crate::AnimationProperty::MorphWeights,
                AnimationValues::MorphWeights { .. }
            )
    )
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

fn validate_namespaced_key(kind: &str, key: &str) -> Result<()> {
    if key.is_empty() || !(key.contains(':') || key.contains('.')) {
        return invalid(format!("{kind} key {key:?} must be namespaced"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(BaoziError::InvalidScene {
        message: message.into(),
    })
}
