use baozi_core::{
    Color, Diagnostic, DiagnosticSeverity, Material, Mesh, MetadataMap, Node, Scene,
    SourceLocation, Texture, TextureSource,
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
                "scene nodes={} meshes={} materials={} textures={} animations={} cameras={} lights={}",
                scene.nodes.len(),
                scene.meshes.len(),
                scene.materials.len(),
                scene.textures.len(),
                scene.animations.len(),
                scene.cameras.len(),
                scene.lights.len()
            ),
        );
        line(&mut text, format_args!("root {}", scene.root.0));
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
            "node {index} name={} parent={} children={} meshes={} metadata={}",
            optional_str(node.name.as_deref()),
            optional_id(node.parent.map(|id| id.0)),
            id_list(node.children.iter().map(|id| id.0)),
            id_list(node.meshes.iter().map(|id| id.0)),
            metadata_keys(&node.metadata)
        ),
    );
}

fn write_mesh(text: &mut String, index: usize, mesh: &Mesh, options: SnapshotOptions) {
    line(
        text,
        format_args!(
            "mesh {index} name={} topology={:?} vertices={} indices={} faces={} material={} metadata={} bounds={}",
            optional_str(mesh.name.as_deref()),
            mesh.topology,
            mesh.positions.len(),
            mesh.indices.len(),
            mesh.polygon_face_count()
                .map_or_else(|| "<fixed>".to_owned(), |count| count.to_string()),
            optional_id(mesh.material.map(|id| id.0)),
            metadata_keys(&mesh.metadata),
            bounds(mesh, options.float_precision)
        ),
    );

    write_vec3_rows(text, "positions", &mesh.positions, options);
    write_vec3_rows(text, "normals", &mesh.normals, options);
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
}

fn write_texture(text: &mut String, index: usize, texture: &Texture) {
    line(
        text,
        format_args!(
            "texture {index} name={} source={}",
            optional_str(texture.name.as_deref()),
            texture_source(&texture.source)
        ),
    );
}

fn write_material(text: &mut String, index: usize, material: &Material, precision: usize) {
    line(
        text,
        format_args!(
            "material {index} name={} shading={:?} base={} metallic={} roughness={} emissive={} alpha={:?} double_sided={} textures={} metadata={}",
            optional_str(material.name.as_deref()),
            material.shading_model,
            color(material.base_color, precision),
            f32_value(material.metallic, precision),
            f32_value(material.roughness, precision),
            color(material.emissive, precision),
            material.alpha_mode,
            material.double_sided,
            material.texture_slots.len(),
            metadata_keys(&material.metadata)
        ),
    );
    for (slot_index, slot) in material.texture_slots.iter().enumerate() {
        line(
            text,
            format_args!(
                "  slot {slot_index} texture={} role={:?} color_space={:?} uv_set={} scale={} source_key={}",
                slot.texture.0,
                slot.role,
                slot.color_space,
                slot.uv_set,
                f32_value(slot.scale, precision),
                optional_str(slot.source_key.as_deref())
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

fn color(value: Color, precision: usize) -> String {
    format!(
        "({},{},{},{})",
        f32_value(value.r, precision),
        f32_value(value.g, precision),
        f32_value(value.b, precision),
        f32_value(value.a, precision)
    )
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
