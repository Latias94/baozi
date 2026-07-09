use baozi_core::scene::{Axis, Handedness};
use baozi_core::{
    Aabb, AlphaMode, BaoziError, Camera, CameraProjection, Color, ColorSpace, Diagnostic,
    DiagnosticCode, DiagnosticSeverity, Mat4, Material, MaterialProperty, Mesh, MeshBinding,
    MetadataMap, MetadataValue, Node, PrimitiveTopology, Result, Scene, SceneBuilder, SceneSpace,
    Skin, SourceLocation, Texture, TextureFilterMode, TextureRole, TextureSampler, TextureSlot,
    TextureSource, TextureTransform, TextureWrapMode, Vec2, Vec3, Vec4,
};
use baozi_import::{ExternalReferencePolicy, ImportContext};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use gltf::buffer::Source as BufferSource;
use gltf::image::Source as ImageSource;
use gltf::json;
use gltf::json::validation::Checked;
use gltf::mesh::Mode;
use gltf::texture::{MagFilter, MinFilter, WrappingMode};
use std::panic::{AssertUnwindSafe, catch_unwind};

pub(crate) fn read_gltf(ctx: &mut ImportContext<'_>) -> Result<Scene> {
    let bytes = ctx.read_primary_to_end()?;
    let gltf = safe_gltf(ctx, "glTF document parse", || {
        gltf::Gltf::from_slice(&bytes)
    })?
    .map_err(|error| {
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
                load_data_uri_buffer(ctx, uri, buffer.index())?
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

fn load_data_uri_buffer(
    ctx: &mut ImportContext<'_>,
    uri: &str,
    buffer_index: usize,
) -> Result<Vec<u8>> {
    if uri.len() > ctx.limits().max_string_bytes {
        return Err(BaoziError::LimitExceeded {
            limit: "max_string_bytes",
        });
    }
    let Some(payload) = uri.strip_prefix("data:") else {
        return Err(gltf_parse_error(
            ctx,
            format!("glTF buffer {buffer_index} has an invalid data URI"),
        ));
    };
    let Some((metadata, encoded)) = payload.split_once(',') else {
        return Err(gltf_parse_error(
            ctx,
            format!("glTF buffer {buffer_index} data URI has no comma separator"),
        ));
    };
    if !metadata
        .split(';')
        .any(|part| part.eq_ignore_ascii_case("base64"))
    {
        return Err(BaoziError::FeatureUnsupported {
            format: "gltf",
            feature: format!("buffer {buffer_index} uses a non-base64 data URI"),
        });
    }

    let decoded_len = base64_decoded_len(ctx, encoded, buffer_index)?;
    ctx.check_data_uri_bytes(decoded_len)?;
    let decoded = BASE64_STANDARD.decode(encoded).map_err(|error| {
        gltf_parse_error(
            ctx,
            format!("glTF buffer {buffer_index} data URI has invalid base64: {error}"),
        )
    })?;
    ctx.record_data_uri_bytes(decoded.len() as u64)?;
    Ok(decoded)
}

fn base64_decoded_len(ctx: &ImportContext<'_>, encoded: &str, buffer_index: usize) -> Result<u64> {
    if encoded.is_empty() {
        return Ok(0);
    }
    if encoded.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err(gltf_parse_error(
            ctx,
            format!("glTF buffer {buffer_index} data URI base64 contains whitespace"),
        ));
    }
    if !encoded.len().is_multiple_of(4) {
        return Err(gltf_parse_error(
            ctx,
            format!("glTF buffer {buffer_index} data URI base64 length is not padded"),
        ));
    }
    let padding = encoded
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .count();
    if padding > 2 || encoded[..encoded.len() - padding].contains('=') {
        return Err(gltf_parse_error(
            ctx,
            format!("glTF buffer {buffer_index} data URI base64 padding is invalid"),
        ));
    }
    let groups = (encoded.len() / 4) as u64;
    groups
        .checked_mul(3)
        .and_then(|bytes| bytes.checked_sub(padding as u64))
        .ok_or(BaoziError::LimitExceeded {
            limit: "max_data_uri_bytes",
        })
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

fn gltf_parse_error(ctx: &ImportContext<'_>, message: impl Into<String>) -> BaoziError {
    BaoziError::parse(ctx.source().to_string(), None, message)
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
    let mut total_faces = 0usize;
    for mesh in gltf.meshes() {
        let mut primitive_meshes = Vec::new();
        for primitive in mesh.primitives() {
            let mesh_id = add_primitive_mesh(
                ctx,
                &mut builder,
                gltf,
                &material_ids,
                buffers,
                &mesh,
                primitive,
                &mut total_vertices,
                &mut total_faces,
            )?;
            primitive_meshes.push(mesh_id);
        }
        mesh_ids_by_gltf_mesh.push(primitive_meshes);
    }

    add_cameras(ctx, &mut builder, gltf);
    add_unsupported_domain_diagnostics(ctx, gltf);
    let node_imports = add_scene_nodes(ctx, &mut builder, gltf, &mesh_ids_by_gltf_mesh)?;
    let skin_ids = add_skins(ctx, &mut builder, gltf, buffers, &node_imports.node_ids)?;
    attach_node_skins(ctx, &mut builder, &node_imports.node_skins, &skin_ids)?;

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

#[derive(Debug, Clone, Copy)]
struct PrimitiveContract {
    mode: Mode,
    position_count: usize,
    material_index: Option<usize>,
}

fn validate_primitive_contract(
    ctx: &ImportContext<'_>,
    gltf: &gltf::Gltf,
    mesh_index: usize,
    primitive_index: usize,
    material_count: usize,
    total_faces: &mut usize,
) -> Result<PrimitiveContract> {
    let primitive = json_primitive(ctx, gltf, mesh_index, primitive_index)?;
    let mode = checked_primitive_mode(ctx, mesh_index, primitive_index, primitive.mode)?;
    let position_index = primitive
        .attributes
        .get(&Checked::Valid(json::mesh::Semantic::Positions))
        .ok_or_else(|| {
            BaoziError::parse(
                ctx.source().to_string(),
                None,
                format!(
                    "glTF mesh {mesh_index} primitive {primitive_index} has no POSITION attribute"
                ),
            )
        })?
        .value();
    let position_count = validate_accessor(
        ctx,
        gltf,
        position_index,
        mesh_index,
        primitive_index,
        "POSITION",
        &[json::accessor::ComponentType::F32],
        &[json::accessor::Type::Vec3],
    )?;

    for (semantic, accessor) in &primitive.attributes {
        let label = match semantic {
            Checked::Valid(json::mesh::Semantic::Positions) => continue,
            Checked::Valid(json::mesh::Semantic::Normals) => "NORMAL",
            Checked::Valid(json::mesh::Semantic::Tangents) => "TANGENT",
            Checked::Valid(json::mesh::Semantic::TexCoords(_)) => "TEXCOORD",
            Checked::Valid(json::mesh::Semantic::Colors(_)) => "COLOR",
            Checked::Valid(json::mesh::Semantic::Joints(_)) => "JOINTS",
            Checked::Valid(json::mesh::Semantic::Weights(_)) => "WEIGHTS",
            Checked::Invalid => {
                return Err(BaoziError::parse(
                    ctx.source().to_string(),
                    None,
                    format!(
                        "glTF mesh {mesh_index} primitive {primitive_index} has an invalid attribute semantic"
                    ),
                ));
            }
        };
        let (components, dimensions): (&[json::accessor::ComponentType], &[json::accessor::Type]) =
            match semantic {
                Checked::Valid(json::mesh::Semantic::Normals) => (
                    &[json::accessor::ComponentType::F32],
                    &[json::accessor::Type::Vec3],
                ),
                Checked::Valid(json::mesh::Semantic::Tangents) => (
                    &[json::accessor::ComponentType::F32],
                    &[json::accessor::Type::Vec4],
                ),
                Checked::Valid(json::mesh::Semantic::TexCoords(_)) => (
                    &[
                        json::accessor::ComponentType::F32,
                        json::accessor::ComponentType::U8,
                        json::accessor::ComponentType::U16,
                    ],
                    &[json::accessor::Type::Vec2],
                ),
                Checked::Valid(json::mesh::Semantic::Colors(_)) => (
                    &[
                        json::accessor::ComponentType::F32,
                        json::accessor::ComponentType::U8,
                        json::accessor::ComponentType::U16,
                    ],
                    &[json::accessor::Type::Vec3, json::accessor::Type::Vec4],
                ),
                Checked::Valid(json::mesh::Semantic::Joints(_)) => (
                    &[
                        json::accessor::ComponentType::U8,
                        json::accessor::ComponentType::U16,
                    ],
                    &[json::accessor::Type::Vec4],
                ),
                Checked::Valid(json::mesh::Semantic::Weights(_)) => (
                    &[
                        json::accessor::ComponentType::F32,
                        json::accessor::ComponentType::U8,
                        json::accessor::ComponentType::U16,
                    ],
                    &[json::accessor::Type::Vec4],
                ),
                _ => continue,
            };
        validate_accessor(
            ctx,
            gltf,
            accessor.value(),
            mesh_index,
            primitive_index,
            label,
            components,
            dimensions,
        )?;
    }

    let index_count = if let Some(index_accessor) = primitive.indices {
        Some(validate_accessor(
            ctx,
            gltf,
            index_accessor.value(),
            mesh_index,
            primitive_index,
            "indices",
            &[
                json::accessor::ComponentType::U8,
                json::accessor::ComponentType::U16,
                json::accessor::ComponentType::U32,
            ],
            &[json::accessor::Type::Scalar],
        )?)
    } else {
        None
    };
    let primitive_faces = match mode {
        Mode::Triangles => index_count.unwrap_or(position_count) / 3,
        Mode::Points | Mode::Lines => 0,
        Mode::LineLoop | Mode::LineStrip | Mode::TriangleStrip | Mode::TriangleFan => 0,
    };
    *total_faces = total_faces
        .checked_add(primitive_faces)
        .ok_or(BaoziError::LimitExceeded { limit: "max_faces" })?;
    if *total_faces > ctx.limits().max_faces {
        return Err(BaoziError::LimitExceeded { limit: "max_faces" });
    }

    let material_index = primitive.material.map(|material| material.value());
    if let Some(material_index) = material_index
        && material_index >= material_count
    {
        return Err(BaoziError::parse(
            ctx.source().to_string(),
            None,
            format!(
                "glTF mesh {mesh_index} primitive {primitive_index} material reference is out of range"
            ),
        ));
    }

    Ok(PrimitiveContract {
        mode,
        position_count,
        material_index,
    })
}

fn json_primitive<'a>(
    ctx: &ImportContext<'_>,
    gltf: &'a gltf::Gltf,
    mesh_index: usize,
    primitive_index: usize,
) -> Result<&'a json::mesh::Primitive> {
    gltf.as_json()
        .meshes
        .get(mesh_index)
        .and_then(|mesh| mesh.primitives.get(primitive_index))
        .ok_or_else(|| {
            BaoziError::parse(
                ctx.source().to_string(),
                None,
                format!("glTF mesh {mesh_index} primitive {primitive_index} is out of range"),
            )
        })
}

fn checked_primitive_mode(
    ctx: &ImportContext<'_>,
    mesh_index: usize,
    primitive_index: usize,
    mode: Checked<json::mesh::Mode>,
) -> Result<Mode> {
    match mode {
        Checked::Valid(json::mesh::Mode::Points) => Ok(Mode::Points),
        Checked::Valid(json::mesh::Mode::Lines) => Ok(Mode::Lines),
        Checked::Valid(json::mesh::Mode::LineLoop) => Ok(Mode::LineLoop),
        Checked::Valid(json::mesh::Mode::LineStrip) => Ok(Mode::LineStrip),
        Checked::Valid(json::mesh::Mode::Triangles) => Ok(Mode::Triangles),
        Checked::Valid(json::mesh::Mode::TriangleStrip) => Ok(Mode::TriangleStrip),
        Checked::Valid(json::mesh::Mode::TriangleFan) => Ok(Mode::TriangleFan),
        Checked::Invalid => Err(BaoziError::parse(
            ctx.source().to_string(),
            None,
            format!(
                "glTF mesh {mesh_index} primitive {primitive_index} has an invalid primitive mode"
            ),
        )),
    }
}

fn validate_accessor(
    ctx: &ImportContext<'_>,
    gltf: &gltf::Gltf,
    accessor_index: usize,
    mesh_index: usize,
    primitive_index: usize,
    label: &str,
    components: &[json::accessor::ComponentType],
    dimensions: &[json::accessor::Type],
) -> Result<usize> {
    let accessor = gltf
        .as_json()
        .accessors
        .get(accessor_index)
        .ok_or_else(|| {
            BaoziError::parse(
                ctx.source().to_string(),
                None,
                format!("glTF mesh {mesh_index} primitive {primitive_index} {label} accessor reference is out of range"),
            )
        })?;
    let component = match accessor.component_type {
        Checked::Valid(component) => component.0,
        Checked::Invalid => {
            return Err(BaoziError::parse(
                ctx.source().to_string(),
                None,
                format!(
                    "glTF mesh {mesh_index} primitive {primitive_index} {label} accessor has an invalid component type"
                ),
            ));
        }
    };
    let dimension = match accessor.type_ {
        Checked::Valid(dimension) => dimension,
        Checked::Invalid => {
            return Err(BaoziError::parse(
                ctx.source().to_string(),
                None,
                format!(
                    "glTF mesh {mesh_index} primitive {primitive_index} {label} accessor has an invalid type"
                ),
            ));
        }
    };
    if !components.contains(&component) || !dimensions.contains(&dimension) {
        return Err(BaoziError::parse(
            ctx.source().to_string(),
            None,
            format!(
                "glTF mesh {mesh_index} primitive {primitive_index} {label} accessor type is unsupported"
            ),
        ));
    }
    usize::try_from(accessor.count.0).map_err(|_| BaoziError::LimitExceeded {
        limit: "max_vertices",
    })
}

fn safe_gltf<T>(
    ctx: &ImportContext<'_>,
    operation: &'static str,
    f: impl FnOnce() -> T,
) -> Result<T> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|_| {
        BaoziError::parse(
            ctx.source().to_string(),
            None,
            format!("{operation} panicked on malformed glTF input"),
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn add_primitive_mesh(
    ctx: &mut ImportContext<'_>,
    builder: &mut SceneBuilder,
    gltf: &gltf::Gltf,
    material_ids: &[baozi_core::MaterialId],
    buffers: &[Vec<u8>],
    mesh: &gltf::Mesh<'_>,
    primitive: gltf::Primitive<'_>,
    total_vertices: &mut usize,
    total_faces: &mut usize,
) -> Result<baozi_core::MeshId> {
    let contract = validate_primitive_contract(
        ctx,
        gltf,
        mesh.index(),
        primitive.index(),
        material_ids.len(),
        total_faces,
    )?;
    let topology = match map_topology(contract.mode) {
        Some(topology) => topology,
        None => {
            return Err(BaoziError::FeatureUnsupported {
                format: "gltf",
                feature: format!("primitive mode {:?} is not implemented yet", contract.mode),
            });
        }
    };

    *total_vertices =
        total_vertices
            .checked_add(contract.position_count)
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
    let positions: Vec<_> = safe_gltf(ctx, "glTF POSITION reader", || {
        reader
            .read_positions()
            .map(|positions| positions.map(vec3).collect::<Vec<_>>())
    })?
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
    })?;
    let bounds = compute_bounds(&positions);
    let normals: Vec<_> = safe_gltf(ctx, "glTF NORMAL reader", || {
        reader
            .read_normals()
            .map(|iter| iter.map(vec3).collect::<Vec<_>>())
    })?
    .unwrap_or_default();
    let tangents: Vec<_> = safe_gltf(ctx, "glTF TANGENT reader", || {
        reader
            .read_tangents()
            .map(|iter| iter.map(vec4).collect::<Vec<_>>())
    })?
    .unwrap_or_default();
    let indices: Vec<_> = safe_gltf(ctx, "glTF indices reader", || {
        reader
            .read_indices()
            .map(|indices| indices.into_u32().collect::<Vec<_>>())
    })?
    .unwrap_or_default();

    let mut texcoords = Vec::new();
    for set in 0..8 {
        let Some(values) = safe_gltf(ctx, "glTF TEXCOORD reader", || {
            reader
                .read_tex_coords(set)
                .map(|values| values.into_f32().map(vec2).collect::<Vec<_>>())
        })?
        else {
            break;
        };
        texcoords.push(values);
    }
    let mut colors = Vec::new();
    for set in 0..8 {
        let Some(values) = safe_gltf(ctx, "glTF COLOR reader", || {
            reader
                .read_colors(set)
                .map(|values| values.into_rgba_f32().map(color).collect::<Vec<_>>())
        })?
        else {
            break;
        };
        colors.push(values);
    }
    let joint_indices = safe_gltf(ctx, "glTF JOINTS reader", || {
        reader
            .read_joints(0)
            .map(|values| values.into_u16().collect::<Vec<_>>())
    })?
    .unwrap_or_default();
    let joint_weights = safe_gltf(ctx, "glTF WEIGHTS reader", || {
        reader
            .read_weights(0)
            .map(|values| values.into_f32().collect::<Vec<_>>())
    })?
    .unwrap_or_default();

    if safe_gltf(ctx, "glTF morph target iterator", || {
        primitive.morph_targets().next().is_some()
    })? {
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

    let material = contract
        .material_index
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
) -> Result<NodeImportMaps> {
    let mut imports = NodeImportMaps {
        node_ids: vec![None; gltf.nodes().count()],
        node_skins: Vec::new(),
    };
    let root = builder.root();
    if let Some(scene) = gltf.default_scene().or_else(|| gltf.scenes().next()) {
        for node in scene.nodes() {
            add_node_recursive(
                ctx,
                builder,
                root,
                node,
                mesh_ids_by_gltf_mesh,
                &mut imports,
            )?;
        }
    } else {
        for node in gltf.nodes() {
            add_node_recursive(
                ctx,
                builder,
                root,
                node,
                mesh_ids_by_gltf_mesh,
                &mut imports,
            )?;
        }
    }
    Ok(imports)
}

#[derive(Debug, Default)]
struct NodeImportMaps {
    node_ids: Vec<Option<baozi_core::NodeId>>,
    node_skins: Vec<(baozi_core::NodeId, usize)>,
}

fn add_node_recursive(
    ctx: &mut ImportContext<'_>,
    builder: &mut SceneBuilder,
    parent: baozi_core::NodeId,
    node: gltf::Node<'_>,
    mesh_ids_by_gltf_mesh: &[Vec<baozi_core::MeshId>],
    imports: &mut NodeImportMaps,
) -> Result<baozi_core::NodeId> {
    let mesh = safe_gltf(ctx, "glTF node mesh", || node.mesh())?;
    let mesh_bindings = mesh
        .and_then(|mesh| mesh_ids_by_gltf_mesh.get(mesh.index()).cloned())
        .map(|meshes| meshes.into_iter().map(MeshBinding::new).collect())
        .unwrap_or_default();
    let camera = safe_gltf(ctx, "glTF node camera", || node.camera())?
        .map(|camera| baozi_core::CameraId::new(camera.index() as u32));

    let node_id = builder.add_child_node(
        parent,
        Node {
            name: optional_name(node.name(), ctx.limits().max_string_bytes)?,
            transform: Mat4 {
                cols: node.transform().matrix(),
            },
            mesh_bindings,
            camera,
            ..Node::default()
        },
    )?;
    if let Some(slot) = imports.node_ids.get_mut(node.index()) {
        *slot = Some(node_id);
    }
    if let Some(skin) = safe_gltf(ctx, "glTF node skin", || node.skin())? {
        imports.node_skins.push((node_id, skin.index()));
    }
    let children = safe_gltf(ctx, "glTF node children", || {
        node.children().collect::<Vec<_>>()
    })?;
    for child in children {
        add_node_recursive(ctx, builder, node_id, child, mesh_ids_by_gltf_mesh, imports)?;
    }
    Ok(node_id)
}

fn add_skins(
    ctx: &mut ImportContext<'_>,
    builder: &mut SceneBuilder,
    gltf: &gltf::Gltf,
    buffers: &[Vec<u8>],
    node_ids: &[Option<baozi_core::NodeId>],
) -> Result<Vec<baozi_core::SkinId>> {
    let mut skin_ids = Vec::with_capacity(gltf.skins().count());
    for skin in gltf.skins() {
        let joints = safe_gltf(ctx, "glTF skin joints", || {
            skin.joints().collect::<Vec<_>>()
        })?;
        let mut mapped_joints = Vec::with_capacity(joints.len());
        for joint in joints {
            let Some(Some(node_id)) = node_ids.get(joint.index()) else {
                return Err(BaoziError::parse(
                    ctx.source().to_string(),
                    None,
                    format!(
                        "glTF skin {} references joint node {} outside the imported scene",
                        skin.index(),
                        joint.index()
                    ),
                ));
            };
            mapped_joints.push(*node_id);
        }

        let skeleton_root = match safe_gltf(ctx, "glTF skin skeleton root", || skin.skeleton())? {
            Some(root) => match node_ids.get(root.index()).copied().flatten() {
                Some(node_id) => Some(node_id),
                None => {
                    return Err(BaoziError::parse(
                        ctx.source().to_string(),
                        None,
                        format!(
                            "glTF skin {} references skeleton root node {} outside the imported scene",
                            skin.index(),
                            root.index()
                        ),
                    ));
                }
            },
            None => None,
        };

        let reader = skin.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));
        let inverse_bind_matrices = safe_gltf(ctx, "glTF inverse bind matrix reader", || {
            reader
                .read_inverse_bind_matrices()
                .map(|matrices| matrices.map(|cols| Mat4 { cols }).collect::<Vec<_>>())
                .unwrap_or_default()
        })?;

        let skin_id = builder.add_skin(Skin {
            name: optional_name(skin.name(), ctx.limits().max_string_bytes)?,
            joints: mapped_joints,
            inverse_bind_matrices,
            skeleton_root,
            metadata: MetadataMap::new(),
        });
        skin_ids.push(skin_id);
    }
    Ok(skin_ids)
}

fn attach_node_skins(
    ctx: &ImportContext<'_>,
    builder: &mut SceneBuilder,
    node_skins: &[(baozi_core::NodeId, usize)],
    skin_ids: &[baozi_core::SkinId],
) -> Result<()> {
    for (node_id, skin_index) in node_skins {
        let Some(skin_id) = skin_ids.get(*skin_index).copied() else {
            return Err(BaoziError::parse(
                ctx.source().to_string(),
                None,
                format!(
                    "glTF node {} references skin {} out of range",
                    node_id.as_u32(),
                    skin_index
                ),
            ));
        };
        let Some(node) = builder.node_mut(*node_id) else {
            return Err(BaoziError::InvalidScene {
                message: format!("node {} is out of range", node_id.as_u32()),
            });
        };
        for binding in &mut node.mesh_bindings {
            binding.skin = Some(skin_id);
        }
    }
    Ok(())
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
            "animation channels are not imported by the glTF MVP",
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
