use baozi_core::{
    Animation, AnimationValues, Camera, CameraProjection, Color, Diagnostic, DiagnosticSeverity,
    Light, Material, Mesh, MeshBinding, MetadataMap, Node, Scene, Skin, SourceLocation, Texture,
    TextureSource, VertexAttributeData,
};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotOptions {
    pub float_precision: usize,
    pub max_vertices_per_mesh: usize,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            float_precision: 6,
            max_vertices_per_mesh: 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneSnapshot {
    text: String,
}

impl SceneSnapshot {
    pub fn from_scene(scene: &Scene) -> Self {
        Self::from_scene_with_options(scene, &[], SnapshotOptions::default())
    }

    pub fn from_scene_with_diagnostics(scene: &Scene, diagnostics: &[Diagnostic]) -> Self {
        Self::from_scene_with_options(scene, diagnostics, SnapshotOptions::default())
    }

    pub fn from_scene_with_options(
        scene: &Scene,
        diagnostics: &[Diagnostic],
        options: SnapshotOptions,
    ) -> Self {
        let mut text = String::new();
        line(&mut text, format_args!("baozi-scene-snapshot-v1"));
        line(
            &mut text,
            format_args!(
                "scene nodes={} meshes={} materials={} textures={} animations={} cameras={} lights={} skins={}",
                scene.nodes.len(),
                scene.meshes.len(),
                scene.materials.len(),
                scene.textures.len(),
                scene.animations.len(),
                scene.cameras.len(),
                scene.lights.len(),
                scene.skins.len()
            ),
        );
        line(&mut text, format_args!("root {}", scene.root.as_u32()));
        line(
            &mut text,
            format_args!(
                "space handedness={:?} up={:?} front={:?} unit={}",
                scene.space.handedness,
                scene.space.up_axis,
                scene.space.front_axis,
                optional_f32(scene.space.unit_scale_to_meters, options.float_precision)
            ),
        );
        line(
            &mut text,
            format_args!("metadata keys={}", metadata_keys(&scene.metadata)),
        );

        for (index, node) in scene.nodes.iter().enumerate() {
            write_node(&mut text, index, node);
        }
        for (index, mesh) in scene.meshes.iter().enumerate() {
            write_mesh(&mut text, index, mesh, options);
        }
        for (index, texture) in scene.textures.iter().enumerate() {
            write_texture(&mut text, index, texture);
        }
        for (index, material) in scene.materials.iter().enumerate() {
            write_material(&mut text, index, material, options.float_precision);
        }
        for (index, skin) in scene.skins.iter().enumerate() {
            write_skin(&mut text, index, skin);
        }
        for (index, camera) in scene.cameras.iter().enumerate() {
            write_camera(&mut text, index, camera, options.float_precision);
        }
        for (index, light) in scene.lights.iter().enumerate() {
            write_light(&mut text, index, light, options.float_precision);
        }
        for (index, animation) in scene.animations.iter().enumerate() {
            write_animation(&mut text, index, animation);
        }
        write_diagnostics(&mut text, diagnostics);

        Self { text }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn into_string(self) -> String {
        self.text
    }
}

impl fmt::Display for SceneSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

fn write_node(text: &mut String, index: usize, node: &Node) {
    line(
        text,
        format_args!(
            "node {index} name={} parent={} children={} meshes={} camera={} light={} metadata={}",
            optional_str(node.name.as_deref()),
            optional_id(node.parent.map(|id| id.as_u32())),
            id_list(node.children.iter().map(|id| id.as_u32())),
            mesh_bindings(&node.mesh_bindings),
            optional_id(node.camera.map(|id| id.as_u32())),
            optional_id(node.light.map(|id| id.as_u32())),
            metadata_keys(&node.metadata)
        ),
    );
}

fn write_mesh(text: &mut String, index: usize, mesh: &Mesh, options: SnapshotOptions) {
    line(
        text,
        format_args!(
            "mesh {index} name={} topology={:?} vertices={} indices={} faces={} material={} joints={} morph_targets={} custom_attributes={} metadata={} bounds={}",
            optional_str(mesh.name.as_deref()),
            mesh.topology,
            mesh.positions.len(),
            mesh.indices.len(),
            mesh.polygon_face_count()
                .map_or_else(|| "<fixed>".to_owned(), |count| count.to_string()),
            optional_id(mesh.material.map(|id| id.as_u32())),
            mesh.joint_indices.len(),
            mesh.morph_targets.len(),
            mesh.custom_attributes.len(),
            metadata_keys(&mesh.metadata),
            bounds(mesh, options.float_precision)
        ),
    );

    write_vec3_rows(text, "positions", &mesh.positions, options);
    write_vec3_rows(text, "normals", &mesh.normals, options);
    write_vec4_rows(text, "tangents", &mesh.tangents, options);
    for (channel, texcoords) in mesh.texcoords.iter().enumerate() {
        write_vec2_rows(text, &format!("texcoords[{channel}]"), texcoords, options);
    }
    for (channel, colors) in mesh.colors.iter().enumerate() {
        write_color_rows(text, &format!("colors[{channel}]"), colors, options);
    }
    line(text, format_args!("  indices={}", u32_list(&mesh.indices)));
    line(
        text,
        format_args!(
            "  face_vertex_counts={}",
            u32_list(&mesh.face_vertex_counts)
        ),
    );
    for (target_index, target) in mesh.morph_targets.iter().enumerate() {
        line(
            text,
            format_args!(
                "  morph_target {target_index} name={} positions={} normals={} tangents={} metadata={}",
                optional_str(target.name.as_deref()),
                target.positions.len(),
                target.normals.len(),
                target.tangents.len(),
                metadata_keys(&target.metadata)
            ),
        );
    }
    for (attribute_index, attribute) in mesh.custom_attributes.iter().enumerate() {
        line(
            text,
            format_args!(
                "  custom_attribute {attribute_index} name={} semantic={:?} len={} metadata={}",
                attribute.name,
                attribute.semantic,
                attribute.data.len(),
                metadata_keys(&attribute.metadata)
            ),
        );
        write_attribute_preview(text, attribute_index, &attribute.data, options);
    }
}

fn write_texture(text: &mut String, index: usize, texture: &Texture) {
    line(
        text,
        format_args!(
            "texture {index} name={} source={} sampler=mag:{:?},min:{:?},wrap:{:?}/{:?}/{:?} metadata={}",
            optional_str(texture.name.as_deref()),
            texture_source(&texture.source),
            texture.sampler.mag_filter,
            texture.sampler.min_filter,
            texture.sampler.wrap_s,
            texture.sampler.wrap_t,
            texture.sampler.wrap_r,
            metadata_keys(&texture.metadata)
        ),
    );
}

fn write_material(text: &mut String, index: usize, material: &Material, precision: usize) {
    line(
        text,
        format_args!(
            "material {index} name={} shading={:?} base={} metallic={} roughness={} emissive={} alpha={:?} double_sided={} textures={} properties={} metadata={}",
            optional_str(material.name.as_deref()),
            material.shading_model,
            color(material.base_color, precision),
            f32_value(material.metallic, precision),
            f32_value(material.roughness, precision),
            color(material.emissive, precision),
            material.alpha_mode,
            material.double_sided,
            material.texture_slots.len(),
            property_keys(material),
            metadata_keys(&material.metadata)
        ),
    );
    for (slot_index, slot) in material.texture_slots.iter().enumerate() {
        line(
            text,
            format_args!(
                "  slot {slot_index} texture={} role={:?} color_space={:?} uv_set={} scale={} transform=offset{} rotation={} scale{} texcoord={} source_key={}",
                slot.texture.as_u32(),
                slot.role,
                slot.color_space,
                slot.uv_set,
                f32_value(slot.scale, precision),
                vec2(slot.transform.offset, precision),
                f32_value(slot.transform.rotation_radians, precision),
                vec2(slot.transform.scale, precision),
                optional_id(slot.transform.texcoord),
                optional_str(slot.source_key.as_deref())
            ),
        );
    }
}

fn write_skin(text: &mut String, index: usize, skin: &Skin) {
    line(
        text,
        format_args!(
            "skin {index} name={} joints={} inverse_bind_matrices={} skeleton_root={} metadata={}",
            optional_str(skin.name.as_deref()),
            id_list(skin.joints.iter().map(|id| id.as_u32())),
            skin.inverse_bind_matrices.len(),
            optional_id(skin.skeleton_root.map(|id| id.as_u32())),
            metadata_keys(&skin.metadata)
        ),
    );
}

fn mesh_bindings(bindings: &[MeshBinding]) -> String {
    if bindings.is_empty() {
        return "[]".to_owned();
    }
    let values = bindings
        .iter()
        .map(|binding| match binding.skin {
            Some(skin) => format!("mesh:{} skin:{}", binding.mesh.as_u32(), skin.as_u32()),
            None => format!("mesh:{} skin:-", binding.mesh.as_u32()),
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{values}]")
}

fn write_camera(text: &mut String, index: usize, camera: &Camera, precision: usize) {
    line(
        text,
        format_args!(
            "camera {index} name={} projection={} metadata={}",
            optional_str(camera.name.as_deref()),
            camera_projection(&camera.projection, precision),
            metadata_keys(&camera.metadata)
        ),
    );
}

fn write_light(text: &mut String, index: usize, light: &Light, precision: usize) {
    line(
        text,
        format_args!(
            "light {index} name={} kind={:?} color={} intensity={} range={} cones={}/{} metadata={}",
            optional_str(light.name.as_deref()),
            light.kind,
            color(light.color, precision),
            f32_value(light.intensity, precision),
            optional_f32(light.range, precision),
            optional_f32(light.inner_cone_angle, precision),
            optional_f32(light.outer_cone_angle, precision),
            metadata_keys(&light.metadata)
        ),
    );
}

fn write_animation(text: &mut String, index: usize, animation: &Animation) {
    line(
        text,
        format_args!(
            "animation {index} name={} channels={} metadata={}",
            optional_str(animation.name.as_deref()),
            animation.channels.len(),
            metadata_keys(&animation.metadata)
        ),
    );
    for (channel_index, channel) in animation.channels.iter().enumerate() {
        line(
            text,
            format_args!(
                "  channel {channel_index} node={} property={:?} interpolation={:?} times={} values={}",
                channel.target.node.as_u32(),
                channel.target.property,
                channel.interpolation,
                channel.times_seconds.len(),
                animation_value_count(&channel.values)
            ),
        );
    }
}

fn write_diagnostics(text: &mut String, diagnostics: &[Diagnostic]) {
    let mut sorted = diagnostics.to_vec();
    sorted.sort_by(|left, right| {
        severity_rank(left.severity)
            .cmp(&severity_rank(right.severity))
            .then_with(|| left.code.0.cmp(right.code.0))
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| location_text(left.location).cmp(&location_text(right.location)))
            .then_with(|| left.message.cmp(&right.message))
    });

    line(text, format_args!("diagnostics count={}", sorted.len()));
    for diagnostic in sorted {
        line(
            text,
            format_args!(
                "diagnostic severity={:?} code={} source={} location={} message={}",
                diagnostic.severity,
                diagnostic.code.0,
                optional_str(diagnostic.source.as_deref()),
                location_text(diagnostic.location),
                diagnostic.message
            ),
        );
    }
}

fn write_vec3_rows(
    text: &mut String,
    name: &str,
    values: &[baozi_core::Vec3],
    options: SnapshotOptions,
) {
    let shown = values.len().min(options.max_vertices_per_mesh);
    line(
        text,
        format_args!("  {name} count={} shown={shown}", values.len()),
    );
    for (index, value) in values.iter().take(shown).enumerate() {
        line(
            text,
            format_args!(
                "    {name}[{index}]={}",
                vec3(*value, options.float_precision)
            ),
        );
    }
}

fn write_vec4_rows(
    text: &mut String,
    name: &str,
    values: &[baozi_core::Vec4],
    options: SnapshotOptions,
) {
    let shown = values.len().min(options.max_vertices_per_mesh);
    line(
        text,
        format_args!("  {name} count={} shown={shown}", values.len()),
    );
    for (index, value) in values.iter().take(shown).enumerate() {
        line(
            text,
            format_args!(
                "    {name}[{index}]={}",
                vec4(*value, options.float_precision)
            ),
        );
    }
}

fn write_vec2_rows(
    text: &mut String,
    name: &str,
    values: &[baozi_core::Vec2],
    options: SnapshotOptions,
) {
    let shown = values.len().min(options.max_vertices_per_mesh);
    line(
        text,
        format_args!("  {name} count={} shown={shown}", values.len()),
    );
    for (index, value) in values.iter().take(shown).enumerate() {
        line(
            text,
            format_args!(
                "    {name}[{index}]={}",
                vec2(*value, options.float_precision)
            ),
        );
    }
}

fn write_attribute_preview(
    text: &mut String,
    attribute_index: usize,
    data: &VertexAttributeData,
    options: SnapshotOptions,
) {
    let shown = data.len().min(options.max_vertices_per_mesh);
    match data {
        VertexAttributeData::F32(values) => {
            for (index, value) in values.iter().take(shown).enumerate() {
                line(
                    text,
                    format_args!(
                        "    custom_attribute[{attribute_index}][{index}]={}",
                        f32_value(*value, options.float_precision)
                    ),
                );
            }
        }
        VertexAttributeData::Vec2(values) => {
            for (index, value) in values.iter().take(shown).enumerate() {
                line(
                    text,
                    format_args!(
                        "    custom_attribute[{attribute_index}][{index}]={}",
                        vec2(*value, options.float_precision)
                    ),
                );
            }
        }
        VertexAttributeData::Vec3(values) => {
            for (index, value) in values.iter().take(shown).enumerate() {
                line(
                    text,
                    format_args!(
                        "    custom_attribute[{attribute_index}][{index}]={}",
                        vec3(*value, options.float_precision)
                    ),
                );
            }
        }
        VertexAttributeData::Vec4(values) => {
            for (index, value) in values.iter().take(shown).enumerate() {
                line(
                    text,
                    format_args!(
                        "    custom_attribute[{attribute_index}][{index}]={}",
                        vec4(*value, options.float_precision)
                    ),
                );
            }
        }
        VertexAttributeData::U16x4(values) => {
            for (index, value) in values.iter().take(shown).enumerate() {
                line(
                    text,
                    format_args!(
                        "    custom_attribute[{attribute_index}][{index}]=[{},{},{},{}]",
                        value[0], value[1], value[2], value[3]
                    ),
                );
            }
        }
        VertexAttributeData::U32(values) => {
            for (index, value) in values.iter().take(shown).enumerate() {
                line(
                    text,
                    format_args!("    custom_attribute[{attribute_index}][{index}]={value}"),
                );
            }
        }
        VertexAttributeData::I32(values) => {
            for (index, value) in values.iter().take(shown).enumerate() {
                line(
                    text,
                    format_args!("    custom_attribute[{attribute_index}][{index}]={value}"),
                );
            }
        }
    }
}

fn write_color_rows(text: &mut String, name: &str, values: &[Color], options: SnapshotOptions) {
    let shown = values.len().min(options.max_vertices_per_mesh);
    line(
        text,
        format_args!("  {name} count={} shown={shown}", values.len()),
    );
    for (index, value) in values.iter().take(shown).enumerate() {
        line(
            text,
            format_args!(
                "    {name}[{index}]={}",
                color(*value, options.float_precision)
            ),
        );
    }
}

fn bounds(mesh: &Mesh, precision: usize) -> String {
    mesh.bounds.as_ref().map_or_else(
        || "<none>".to_owned(),
        |bounds| {
            format!(
                "min={} max={}",
                vec3(bounds.min, precision),
                vec3(bounds.max, precision)
            )
        },
    )
}

fn metadata_keys(metadata: &MetadataMap) -> String {
    let keys: Vec<_> = metadata.keys().map(String::as_str).collect();
    format!("[{}]", keys.join(","))
}

fn property_keys(material: &Material) -> String {
    let keys: Vec<_> = material.properties.keys().map(String::as_str).collect();
    format!("[{}]", keys.join(","))
}

fn id_list(values: impl Iterator<Item = u32>) -> String {
    let values: Vec<_> = values.map(|value| value.to_string()).collect();
    format!("[{}]", values.join(","))
}

fn u32_list(values: &[u32]) -> String {
    let values: Vec<_> = values.iter().map(u32::to_string).collect();
    format!("[{}]", values.join(","))
}

fn texture_source(source: &TextureSource) -> String {
    match source {
        TextureSource::External { uri } => format!("external:{uri}"),
        TextureSource::Embedded { bytes, mime_type } => {
            format!(
                "embedded:bytes={} mime={}",
                bytes.len(),
                optional_str(mime_type.as_deref())
            )
        }
    }
}

fn vec2(value: baozi_core::Vec2, precision: usize) -> String {
    format!(
        "({},{})",
        f32_value(value.x, precision),
        f32_value(value.y, precision)
    )
}

fn vec3(value: baozi_core::Vec3, precision: usize) -> String {
    format!(
        "({},{},{})",
        f32_value(value.x, precision),
        f32_value(value.y, precision),
        f32_value(value.z, precision)
    )
}

fn vec4(value: baozi_core::Vec4, precision: usize) -> String {
    format!(
        "({},{},{},{})",
        f32_value(value.x, precision),
        f32_value(value.y, precision),
        f32_value(value.z, precision),
        f32_value(value.w, precision)
    )
}

fn color(value: Color, precision: usize) -> String {
    format!(
        "({},{},{},{})",
        f32_value(value.r, precision),
        f32_value(value.g, precision),
        f32_value(value.b, precision),
        f32_value(value.a, precision)
    )
}

fn camera_projection(projection: &CameraProjection, precision: usize) -> String {
    match projection {
        CameraProjection::Perspective {
            yfov_radians,
            aspect_ratio,
            znear,
            zfar,
        } => format!(
            "perspective:yfov={} aspect={} znear={} zfar={}",
            f32_value(*yfov_radians, precision),
            optional_f32(*aspect_ratio, precision),
            f32_value(*znear, precision),
            optional_f32(*zfar, precision)
        ),
        CameraProjection::Orthographic {
            xmag,
            ymag,
            znear,
            zfar,
        } => format!(
            "orthographic:xmag={} ymag={} znear={} zfar={}",
            f32_value(*xmag, precision),
            f32_value(*ymag, precision),
            f32_value(*znear, precision),
            f32_value(*zfar, precision)
        ),
        CameraProjection::Unknown => "unknown".to_owned(),
    }
}

fn animation_value_count(values: &AnimationValues) -> usize {
    match values {
        AnimationValues::Translations(values) => values.len(),
        AnimationValues::Rotations(values) => values.len(),
        AnimationValues::Scales(values) => values.len(),
        AnimationValues::MorphWeights {
            values,
            weights_per_keyframe,
        } => {
            if *weights_per_keyframe == 0 {
                0
            } else {
                values.len() / weights_per_keyframe
            }
        }
    }
}

fn optional_f32(value: Option<f32>, precision: usize) -> String {
    value.map_or_else(|| "<none>".to_owned(), |value| f32_value(value, precision))
}

fn f32_value(value: f32, precision: usize) -> String {
    let value = if value == 0.0 { 0.0 } else { value };
    format!("{value:.precision$}")
}

fn optional_id(value: Option<u32>) -> String {
    value.map_or_else(|| "<none>".to_owned(), |value| value.to_string())
}

fn optional_str(value: Option<&str>) -> String {
    value.unwrap_or("<none>").to_owned()
}

fn location_text(location: Option<SourceLocation>) -> String {
    location.map_or_else(|| "<none>".to_owned(), |location| location.to_string())
}

fn severity_rank(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Error => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Info => 2,
    }
}

fn line(text: &mut String, args: fmt::Arguments<'_>) {
    use std::fmt::Write as _;

    let _ = text.write_fmt(args);
    text.push('\n');
}
