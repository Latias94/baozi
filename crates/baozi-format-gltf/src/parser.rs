use baozi_core::scene::{Axis, Handedness};
use baozi_core::{
    Aabb, AlphaMode, BaoziError, Camera, CameraProjection, Color, ColorSpace, Diagnostic,
    DiagnosticCode, DiagnosticSeverity, Mat4, Material, MaterialProperty, Mesh, MetadataMap,
    MetadataValue, Node, PrimitiveTopology, Result, Scene, SceneBuilder, SceneSpace,
    SourceLocation, Texture, TextureFilterMode, TextureRole, TextureSampler, TextureSlot,
    TextureSource, TextureTransform, TextureWrapMode, Vec2, Vec3, Vec4,
};
use baozi_import::{ExternalReferencePolicy, ImportContext};
use gltf::buffer::Source as BufferSource;
use gltf::image::Source as ImageSource;
use gltf::mesh::{Mode, Semantic};
use gltf::texture::{MagFilter, MinFilter, WrappingMode};

pub(crate) fn read_gltf(ctx: &mut ImportContext<'_>) -> Result<Scene> {
    let bytes = ctx.read_primary_to_end()?;
    let gltf = gltf::Gltf::from_slice(&bytes).map_err(|error| {
        BaoziError::parse(
            ctx.source().to_string(),
            None,
            format!("invalid glTF: {error}"),
        )
    })?;
    let buffers = load_buffers(ctx, &gltf)?;
    scene_from_document(ctx, &gltf, &buffers)
}

fn load_buffers(ctx: &mut ImportContext<'_>, gltf: &gltf::Gltf) -> Result<Vec<Vec<u8>>> {
    let mut buffers = Vec::with_capacity(gltf.buffers().count());
    for buffer in gltf.buffers() {
        let mut bytes = match buffer.source() {
            BufferSource::Bin => gltf.blob.clone().ok_or_else(|| {
                BaoziError::parse(
                    ctx.source().to_string(),
                    None,
                    format!("glTF buffer {} requires a GLB BIN chunk", buffer.index()),
                )
            })?,
            BufferSource::Uri(uri) if is_data_uri(uri) => {
                return Err(BaoziError::FeatureUnsupported {
                    format: "gltf",
                    feature: "buffer data URIs are not implemented yet".to_owned(),
                });
            }
            BufferSource::Uri(uri) => load_external_buffer(ctx, uri)?,
        };

        if bytes.len() < buffer.length() {
            return Err(BaoziError::parse(
                ctx.source().to_string(),
                None,
                format!(
                    "glTF buffer {} declares {} bytes but only {} bytes are available",
                    buffer.index(),
                    buffer.length(),
                    bytes.len()
                ),
            ));
        }
        bytes.truncate(buffer.length());
        buffers.push(bytes);
    }
    Ok(buffers)
}

fn load_external_buffer(ctx: &mut ImportContext<'_>, uri: &str) -> Result<Vec<u8>> {
    if matches!(
        ctx.io_options().external_references,
        ExternalReferencePolicy::Deny
    ) {
        return Err(BaoziError::parse(
            ctx.source().to_string(),
            None,
            format!("glTF external buffer '{uri}' was denied by ImportOptions"),
        ));
    }
    if uri.len() > ctx.limits().max_string_bytes {
        return Err(BaoziError::LimitExceeded {
            limit: "max_string_bytes",
        });
    }
    let path = ctx.resolve_source_relative(uri)?;
    ctx.read_sidecar_to_end(&path)
}

fn scene_from_document(
    ctx: &mut ImportContext<'_>,
    gltf: &gltf::Gltf,
    buffers: &[Vec<u8>],
) -> Result<Scene> {
    let mut builder = SceneBuilder::new();
    let mut material_ids = Vec::with_capacity(gltf.materials().count());
    for material in gltf.materials() {
        material_ids.push(add_material(ctx, &mut builder, material)?);
    }

    let mut mesh_ids_by_gltf_mesh = Vec::with_capacity(gltf.meshes().count());
    let mut total_vertices = 0usize;
    for mesh in gltf.meshes() {
        let mut primitive_meshes = Vec::new();
        for primitive in mesh.primitives() {
            let mesh_id = add_primitive_mesh(
                ctx,
                &mut builder,
                &material_ids,
                buffers,
                &mesh,
                primitive,
                &mut total_vertices,
            )?;
            primitive_meshes.push(mesh_id);
        }
        mesh_ids_by_gltf_mesh.push(primitive_meshes);
    }

    add_cameras(ctx, &mut builder, gltf);
    add_unsupported_domain_diagnostics(ctx, gltf);
    add_scene_nodes(ctx, &mut builder, gltf, &mesh_ids_by_gltf_mesh)?;

    let mut scene = builder.finish()?;
    scene.space = SceneSpace {
        handedness: Handedness::Right,
        up_axis: Some(Axis::PositiveY),
        front_axis: Some(Axis::NegativeZ),
        unit_scale_to_meters: Some(1.0),
    };
    scene.metadata.insert(
        "gltf:version".to_owned(),
        MetadataValue::String("2.0".to_owned()),
    );
    Ok(scene)
}

fn add_material(
    ctx: &mut ImportContext<'_>,
    builder: &mut SceneBuilder,
    material: gltf::Material<'_>,
) -> Result<baozi_core::MaterialId> {
    let pbr = material.pbr_metallic_roughness();
    let base = pbr.base_color_factor();
    let emissive = material.emissive_factor();
    let mut properties = baozi_core::MaterialPropertyMap::new();
    properties.insert(
        "gltf:material_index".to_owned(),
        MaterialProperty::I64(material.index().unwrap_or_default() as i64),
    );

    let mut material_out = Material {
        name: optional_name(material.name(), ctx.limits().max_string_bytes)?,
        shading_model: baozi_core::ShadingModel::PbrMetallicRoughness,
        base_color: Color::linear_rgba(base[0], base[1], base[2], base[3]),
        metallic: pbr.metallic_factor(),
        roughness: pbr.roughness_factor(),
        emissive: Color::linear_rgba(emissive[0], emissive[1], emissive[2], 1.0),
        alpha_mode: map_alpha_mode(material.alpha_mode()),
        alpha_cutoff: material.alpha_cutoff().unwrap_or(0.5),
        double_sided: material.double_sided(),
        properties,
        ..Material::default()
    };

    if let Some(info) = pbr.base_color_texture() {
        add_texture_slot(
            ctx,
            builder,
            &mut material_out,
            info.texture(),
            info.tex_coord(),
            TextureRole::BaseColor,
            ColorSpace::Srgb,
            1.0,
            "baseColorTexture",
        )?;
    }
    if let Some(info) = pbr.metallic_roughness_texture() {
        add_texture_slot(
            ctx,
            builder,
            &mut material_out,
            info.texture(),
            info.tex_coord(),
            TextureRole::MetallicRoughness,
            ColorSpace::Data,
            1.0,
            "metallicRoughnessTexture",
        )?;
    }
    if let Some(info) = material.normal_texture() {
        add_texture_slot(
            ctx,
            builder,
            &mut material_out,
            info.texture(),
            info.tex_coord(),
            TextureRole::Normal,
            ColorSpace::Data,
            info.scale(),
            "normalTexture",
        )?;
    }
    if let Some(info) = material.occlusion_texture() {
        add_texture_slot(
            ctx,
            builder,
            &mut material_out,
            info.texture(),
            info.tex_coord(),
            TextureRole::Occlusion,
            ColorSpace::Data,
            info.strength(),
            "occlusionTexture",
        )?;
    }
    if let Some(info) = material.emissive_texture() {
        add_texture_slot(
            ctx,
            builder,
            &mut material_out,
            info.texture(),
            info.tex_coord(),
            TextureRole::Emissive,
            ColorSpace::Srgb,
            1.0,
            "emissiveTexture",
        )?;
    }

    Ok(builder.add_material(material_out))
}

#[allow(clippy::too_many_arguments)]
fn add_texture_slot(
    ctx: &mut ImportContext<'_>,
    builder: &mut SceneBuilder,
    material: &mut Material,
    texture: gltf::texture::Texture<'_>,
    tex_coord: u32,
    role: TextureRole,
    color_space: ColorSpace,
    scale: f32,
    source_key: &'static str,
) -> Result<()> {
    let image = texture.source();
    let image_name = optional_name(image.name(), ctx.limits().max_string_bytes)?;
    let source = match image.source() {
        ImageSource::Uri { uri, .. } if is_data_uri(uri) => {
            push_warning(
                ctx,
                ctx.source().to_string(),
                None,
                "gltf.texture_data_uri_ignored",
                format!("texture data URI for {source_key} was ignored"),
            );
            return Ok(());
        }
        ImageSource::Uri { uri, .. } => {
            if uri.len() > ctx.limits().max_string_bytes {
                push_warning(
                    ctx,
                    ctx.source().to_string(),
                    None,
                    "gltf.texture_uri_limit",
                    format!(
                        "texture URI for {source_key} exceeded max_string_bytes and was ignored"
                    ),
                );
                return Ok(());
            }
            let resolved = ctx.resolve_source_relative(uri)?;
            TextureSource::External {
                uri: resolved.to_string(),
            }
        }
        ImageSource::View { mime_type, .. } => {
            push_warning(
                ctx,
                ctx.source().to_string(),
                None,
                "gltf.embedded_texture_ignored",
                format!("embedded {mime_type} texture for {source_key} was ignored"),
            );
            return Ok(());
        }
    };

    let texture_id = builder.add_texture(Texture {
        name: optional_name(texture.name(), ctx.limits().max_string_bytes)?.or(image_name),
        source,
        sampler: map_sampler(texture.sampler()),
        metadata: MetadataMap::new(),
    });
    material.texture_slots.push(TextureSlot {
        texture: texture_id,
        role,
        color_space,
        uv_set: tex_coord,
        scale,
        transform: TextureTransform::default(),
        source_key: Some(source_key.to_owned()),
    });
    material.properties.insert(
        format!("gltf:{source_key}"),
        MaterialProperty::Texture(texture_id),
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_primitive_mesh(
    ctx: &mut ImportContext<'_>,
    builder: &mut SceneBuilder,
    material_ids: &[baozi_core::MaterialId],
    buffers: &[Vec<u8>],
    mesh: &gltf::Mesh<'_>,
    primitive: gltf::Primitive<'_>,
    total_vertices: &mut usize,
) -> Result<baozi_core::MeshId> {
    let topology = match map_topology(primitive.mode()) {
        Some(topology) => topology,
        None => {
            return Err(BaoziError::FeatureUnsupported {
                format: "gltf",
                feature: format!(
                    "primitive mode {:?} is not implemented yet",
                    primitive.mode()
                ),
            });
        }
    };

    let position_accessor = primitive.get(&Semantic::Positions).ok_or_else(|| {
        BaoziError::parse(
            ctx.source().to_string(),
            None,
            format!(
                "glTF mesh {} primitive {} has no POSITION attribute",
                mesh.index(),
                primitive.index()
            ),
        )
    })?;
    *total_vertices = total_vertices
        .checked_add(position_accessor.count())
        .ok_or(BaoziError::LimitExceeded {
            limit: "max_vertices",
        })?;
    if *total_vertices > ctx.limits().max_vertices {
        return Err(BaoziError::LimitExceeded {
            limit: "max_vertices",
        });
    }
    if builder.mesh_count() >= ctx.limits().max_meshes {
        return Err(BaoziError::LimitExceeded {
            limit: "max_meshes",
        });
    }

    let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));
    let positions: Vec<_> = reader
        .read_positions()
        .ok_or_else(|| {
            BaoziError::parse(
                ctx.source().to_string(),
                None,
                format!(
                    "glTF mesh {} primitive {} POSITION accessor could not be read",
                    mesh.index(),
                    primitive.index()
                ),
            )
        })?
        .map(vec3)
        .collect();
    let bounds = compute_bounds(&positions);
    let normals: Vec<_> = reader
        .read_normals()
        .map(|iter| iter.map(vec3).collect())
        .unwrap_or_default();
    let tangents: Vec<_> = reader
        .read_tangents()
        .map(|iter| iter.map(vec4).collect())
        .unwrap_or_default();
    let indices: Vec<_> = reader
        .read_indices()
        .map(|indices| indices.into_u32().collect())
        .unwrap_or_default();

    let mut texcoords = Vec::new();
    for set in 0..8 {
        let Some(values) = reader.read_tex_coords(set) else {
            break;
        };
        texcoords.push(values.into_f32().map(vec2).collect());
    }
    let mut colors = Vec::new();
    for set in 0..8 {
        let Some(values) = reader.read_colors(set) else {
            break;
        };
        colors.push(values.into_rgba_f32().map(color).collect());
    }
    let joint_indices = reader
        .read_joints(0)
        .map(|values| values.into_u16().collect())
        .unwrap_or_default();
    let joint_weights = reader
        .read_weights(0)
        .map(|values| values.into_f32().collect())
        .unwrap_or_default();

    if primitive.morph_targets().next().is_some() {
        push_warning(
            ctx,
            ctx.source().to_string(),
            None,
            "gltf.morph_targets_ignored",
            format!(
                "glTF mesh {} primitive {} morph targets were ignored",
                mesh.index(),
                primitive.index()
            ),
        );
    }

    let mut metadata = MetadataMap::new();
    metadata.insert(
        "gltf:mesh_index".to_owned(),
        MetadataValue::I64(mesh.index() as i64),
    );
    metadata.insert(
        "gltf:primitive_index".to_owned(),
        MetadataValue::I64(primitive.index() as i64),
    );

    let material = primitive
        .material()
        .index()
        .and_then(|index| material_ids.get(index).copied());
    let mesh_name = primitive_name(
        mesh.name(),
        primitive.index(),
        ctx.limits().max_string_bytes,
    )?;
    let mesh_out = Mesh {
        name: mesh_name,
        topology,
        positions,
        normals,
        tangents,
        texcoords,
        colors,
        indices,
        material,
        joint_indices,
        joint_weights,
        bounds,
        metadata,
        ..Mesh::default()
    };
    Ok(builder.add_mesh(mesh_out))
}

fn add_scene_nodes(
    ctx: &mut ImportContext<'_>,
    builder: &mut SceneBuilder,
    gltf: &gltf::Gltf,
    mesh_ids_by_gltf_mesh: &[Vec<baozi_core::MeshId>],
) -> Result<()> {
    let root = builder.root();
    if let Some(scene) = gltf.default_scene().or_else(|| gltf.scenes().next()) {
        for node in scene.nodes() {
            add_node_recursive(ctx, builder, root, node, mesh_ids_by_gltf_mesh)?;
        }
    } else {
        for node in gltf.nodes() {
            add_node_recursive(ctx, builder, root, node, mesh_ids_by_gltf_mesh)?;
        }
    }
    Ok(())
}

fn add_node_recursive(
    ctx: &mut ImportContext<'_>,
    builder: &mut SceneBuilder,
    parent: baozi_core::NodeId,
    node: gltf::Node<'_>,
    mesh_ids_by_gltf_mesh: &[Vec<baozi_core::MeshId>],
) -> Result<baozi_core::NodeId> {
    let meshes = node
        .mesh()
        .and_then(|mesh| mesh_ids_by_gltf_mesh.get(mesh.index()).cloned())
        .unwrap_or_default();
    let camera = node
        .camera()
        .map(|camera| baozi_core::CameraId::new(camera.index() as u32));

    if node.skin().is_some() {
        push_warning(
            ctx,
            ctx.source().to_string(),
            None,
            "gltf.skin_ignored",
            format!("skin on node {} was ignored", node.index()),
        );
    }

    let node_id = builder.add_child_node(
        parent,
        Node {
            name: optional_name(node.name(), ctx.limits().max_string_bytes)?,
            transform: Mat4 {
                cols: node.transform().matrix(),
            },
            meshes,
            camera,
            ..Node::default()
        },
    )?;
    for child in node.children() {
        add_node_recursive(ctx, builder, node_id, child, mesh_ids_by_gltf_mesh)?;
    }
    Ok(node_id)
}

fn add_cameras(ctx: &mut ImportContext<'_>, builder: &mut SceneBuilder, gltf: &gltf::Gltf) {
    for camera in gltf.cameras() {
        let projection = match camera.projection() {
            gltf::camera::Projection::Orthographic(orthographic) => {
                CameraProjection::Orthographic {
                    xmag: orthographic.xmag(),
                    ymag: orthographic.ymag(),
                    znear: orthographic.znear(),
                    zfar: orthographic.zfar(),
                }
            }
            gltf::camera::Projection::Perspective(perspective) => CameraProjection::Perspective {
                yfov_radians: perspective.yfov(),
                aspect_ratio: perspective.aspect_ratio(),
                znear: perspective.znear(),
                zfar: perspective.zfar(),
            },
        };
        let name = match optional_name(camera.name(), ctx.limits().max_string_bytes) {
            Ok(name) => name,
            Err(error) => {
                push_warning(
                    ctx,
                    ctx.source().to_string(),
                    None,
                    "gltf.camera_name_limit",
                    format!("camera name exceeded max_string_bytes and was ignored: {error}"),
                );
                None
            }
        };
        builder.add_camera(Camera {
            name,
            projection,
            ..Camera::default()
        });
    }
}

fn add_unsupported_domain_diagnostics(ctx: &mut ImportContext<'_>, gltf: &gltf::Gltf) {
    if gltf.animations().next().is_some() {
        push_warning(
            ctx,
            ctx.source().to_string(),
            None,
            "gltf.animations_ignored",
            "animation channels are not imported by the static mesh MVP",
        );
    }
    if gltf.skins().next().is_some() {
        push_warning(
            ctx,
            ctx.source().to_string(),
            None,
            "gltf.skins_ignored",
            "skin objects are not imported by the static mesh MVP",
        );
    }
}

fn map_topology(mode: Mode) -> Option<PrimitiveTopology> {
    match mode {
        Mode::Points => Some(PrimitiveTopology::Points),
        Mode::Lines => Some(PrimitiveTopology::Lines),
        Mode::Triangles => Some(PrimitiveTopology::Triangles),
        Mode::LineLoop | Mode::LineStrip | Mode::TriangleStrip | Mode::TriangleFan => None,
    }
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

fn map_alpha_mode(mode: gltf::material::AlphaMode) -> AlphaMode {
    match mode {
        gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
        gltf::material::AlphaMode::Mask => AlphaMode::Mask,
        gltf::material::AlphaMode::Blend => AlphaMode::Blend,
    }
}

fn map_sampler(sampler: gltf::texture::Sampler<'_>) -> TextureSampler {
    TextureSampler {
        mag_filter: sampler.mag_filter().map(map_mag_filter),
        min_filter: sampler.min_filter().map(map_min_filter),
        wrap_s: map_wrap(sampler.wrap_s()),
        wrap_t: map_wrap(sampler.wrap_t()),
        wrap_r: TextureWrapMode::Repeat,
    }
}

fn map_mag_filter(filter: MagFilter) -> TextureFilterMode {
    match filter {
        MagFilter::Nearest => TextureFilterMode::Nearest,
        MagFilter::Linear => TextureFilterMode::Linear,
    }
}

fn map_min_filter(filter: MinFilter) -> TextureFilterMode {
    match filter {
        MinFilter::Nearest => TextureFilterMode::Nearest,
        MinFilter::Linear => TextureFilterMode::Linear,
        MinFilter::NearestMipmapNearest => TextureFilterMode::NearestMipmapNearest,
        MinFilter::LinearMipmapNearest => TextureFilterMode::LinearMipmapNearest,
        MinFilter::NearestMipmapLinear => TextureFilterMode::NearestMipmapLinear,
        MinFilter::LinearMipmapLinear => TextureFilterMode::LinearMipmapLinear,
    }
}

fn map_wrap(wrap: WrappingMode) -> TextureWrapMode {
    match wrap {
        WrappingMode::ClampToEdge => TextureWrapMode::ClampToEdge,
        WrappingMode::MirroredRepeat => TextureWrapMode::MirroredRepeat,
        WrappingMode::Repeat => TextureWrapMode::Repeat,
    }
}

fn primitive_name(
    mesh_name: Option<&str>,
    primitive_index: usize,
    max_string_bytes: usize,
) -> Result<Option<String>> {
    let Some(mesh_name) = mesh_name else {
        return Ok(None);
    };
    let name = if primitive_index == 0 {
        mesh_name.to_owned()
    } else {
        format!("{mesh_name}#{primitive_index}")
    };
    if name.len() > max_string_bytes {
        Err(BaoziError::LimitExceeded {
            limit: "max_string_bytes",
        })
    } else {
        Ok(Some(name))
    }
}

fn optional_name(value: Option<&str>, max_string_bytes: usize) -> Result<Option<String>> {
    value
        .map(|value| {
            if value.len() > max_string_bytes {
                Err(BaoziError::LimitExceeded {
                    limit: "max_string_bytes",
                })
            } else {
                Ok(value.to_owned())
            }
        })
        .transpose()
}

fn is_data_uri(uri: &str) -> bool {
    uri.get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
}

fn vec2(value: [f32; 2]) -> Vec2 {
    Vec2::new(value[0], value[1])
}

fn vec3(value: [f32; 3]) -> Vec3 {
    Vec3::new(value[0], value[1], value[2])
}

fn vec4(value: [f32; 4]) -> Vec4 {
    Vec4::new(value[0], value[1], value[2], value[3])
}

fn color(value: [f32; 4]) -> Color {
    Color::linear_rgba(value[0], value[1], value[2], value[3])
}

fn push_warning(
    ctx: &mut ImportContext<'_>,
    source: String,
    location: Option<SourceLocation>,
    code: &'static str,
    message: impl Into<String>,
) {
    ctx.push_diagnostic(Diagnostic {
        severity: DiagnosticSeverity::Warning,
        code: DiagnosticCode(code),
        source: Some(source),
        location,
        message: message.into(),
    });
}
