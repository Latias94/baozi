use crate::mtl::{self, MaterialLibrary, ParsedMaterial};
use crate::obj::{Face, FaceVertex, ParsedObj};
use baozi_core::{
    Aabb, AlphaMode, BaoziError, ColorSpace, Material, MaterialId, Mesh, MetadataMap,
    MetadataValue, Node, PrimitiveTopology, Result, Scene, SceneBuilder, ShadingModel, Texture,
    TextureRole, TextureSlot, TextureSource, Vec2, Vec3,
};
use baozi_import::ImportContext;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SegmentKey {
    object: Option<String>,
    group: Option<String>,
    material: Option<String>,
    smoothing: Option<String>,
}

impl SegmentKey {
    fn from_face(face: &Face) -> Self {
        Self {
            object: face.object.clone(),
            group: face.group.clone(),
            material: face.material.clone(),
            smoothing: face.smoothing.clone(),
        }
    }

    fn display_name(&self) -> Option<String> {
        self.group
            .clone()
            .or_else(|| self.object.clone())
            .or_else(|| self.material.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct VertexKey {
    position: usize,
    texcoord: Option<usize>,
    normal: Option<usize>,
}

impl From<FaceVertex> for VertexKey {
    fn from(value: FaceVertex) -> Self {
        Self {
            position: value.position,
            texcoord: value.texcoord,
            normal: value.normal,
        }
    }
}

struct SegmentBuilder {
    key: SegmentKey,
    vertex_map: BTreeMap<VertexKey, u32>,
    positions: Vec<Vec3>,
    texcoords: Vec<Vec2>,
    normals: Vec<Vec3>,
    indices: Vec<u32>,
    face_vertex_counts: Vec<u32>,
    has_texcoords: bool,
    has_normals: bool,
    missing_texcoords: bool,
    missing_normals: bool,
    has_polygon: bool,
}

impl SegmentBuilder {
    fn new(key: SegmentKey) -> Self {
        Self {
            key,
            vertex_map: BTreeMap::new(),
            positions: Vec::new(),
            texcoords: Vec::new(),
            normals: Vec::new(),
            indices: Vec::new(),
            face_vertex_counts: Vec::new(),
            has_texcoords: false,
            has_normals: false,
            missing_texcoords: false,
            missing_normals: false,
            has_polygon: false,
        }
    }

    fn push_face(
        &mut self,
        ctx: &ImportContext<'_>,
        parsed: &ParsedObj,
        face: &Face,
    ) -> Result<()> {
        if face.vertices.len() < 3 {
            return Err(BaoziError::parse(
                ctx.source().to_string(),
                None,
                "OBJ face must have at least three vertices",
            ));
        }
        if face.vertices.len() > 3 {
            self.has_polygon = true;
        }
        self.face_vertex_counts.push(face.vertices.len() as u32);

        for vertex in &face.vertices {
            let index = self.vertex_index(ctx, parsed, (*vertex).into())?;
            self.indices.push(index);
        }
        Ok(())
    }

    fn vertex_index(
        &mut self,
        ctx: &ImportContext<'_>,
        parsed: &ParsedObj,
        key: VertexKey,
    ) -> Result<u32> {
        if let Some(index) = self.vertex_map.get(&key) {
            return Ok(*index);
        }

        let next = self
            .positions
            .len()
            .checked_add(1)
            .ok_or(BaoziError::LimitExceeded {
                limit: "max_vertices",
            })?;
        if next > ctx.limits().max_vertices || next > u32::MAX as usize {
            return Err(BaoziError::LimitExceeded {
                limit: "max_vertices",
            });
        }

        let index = self.positions.len() as u32;
        self.positions.push(parsed.positions[key.position]);
        match key.texcoord {
            Some(idx) => self.texcoords.push(parsed.texcoords[idx]),
            None => {
                self.texcoords.push(Vec2::ZERO);
                self.missing_texcoords = true;
            }
        }
        match key.normal {
            Some(idx) => self.normals.push(parsed.normals[idx]),
            None => {
                self.normals.push(Vec3::ZERO);
                self.missing_normals = true;
            }
        }
        self.has_texcoords |= key.texcoord.is_some();
        self.has_normals |= key.normal.is_some();
        self.vertex_map.insert(key, index);
        Ok(index)
    }

    fn warn_partial_attributes(&self, ctx: &mut ImportContext<'_>) {
        let segment = self
            .key
            .display_name()
            .unwrap_or_else(|| "<unnamed>".to_owned());
        if self.has_texcoords && self.missing_texcoords {
            mtl::push_warning(
                ctx,
                ctx.source().to_string(),
                None,
                "obj.partial_texcoord_channel",
                format!(
                    "OBJ segment '{segment}' mixes vertices with and without texture coordinates; the partial texture coordinate channel was omitted"
                ),
            );
        }
        if self.has_normals && self.missing_normals {
            mtl::push_warning(
                ctx,
                ctx.source().to_string(),
                None,
                "obj.partial_normal_channel",
                format!(
                    "OBJ segment '{segment}' mixes vertices with and without normals; the partial normal channel was omitted"
                ),
            );
        }
    }

    fn finish(self, material: Option<MaterialId>) -> Mesh {
        let topology = if self.has_polygon {
            PrimitiveTopology::Polygons
        } else {
            PrimitiveTopology::Triangles
        };
        let face_vertex_counts = if self.has_polygon {
            self.face_vertex_counts
        } else {
            Vec::new()
        };
        let mut metadata = MetadataMap::new();
        if let Some(object) = &self.key.object {
            metadata.insert(
                "obj.object".to_owned(),
                MetadataValue::String(object.clone()),
            );
        }
        if let Some(group) = &self.key.group {
            metadata.insert("obj.group".to_owned(), MetadataValue::String(group.clone()));
        }
        if let Some(material) = &self.key.material {
            metadata.insert(
                "obj.material".to_owned(),
                MetadataValue::String(material.clone()),
            );
        }
        if let Some(smoothing) = &self.key.smoothing {
            metadata.insert(
                "obj.smoothing".to_owned(),
                MetadataValue::String(smoothing.clone()),
            );
        }

        Mesh {
            name: self.key.display_name(),
            topology,
            bounds: compute_bounds(&self.positions),
            positions: self.positions,
            normals: if self.has_normals && !self.missing_normals {
                self.normals
            } else {
                Vec::new()
            },
            texcoords: if self.has_texcoords && !self.missing_texcoords {
                vec![self.texcoords]
            } else {
                Vec::new()
            },
            indices: self.indices,
            face_vertex_counts,
            material,
            metadata,
            ..Mesh::default()
        }
    }
}

pub(crate) fn scene_from_parsed(
    ctx: &mut ImportContext<'_>,
    parsed: ParsedObj,
    library: &MaterialLibrary,
) -> Result<Scene> {
    if parsed.faces.is_empty() {
        return Err(BaoziError::parse(
            ctx.source().to_string(),
            None,
            "OBJ contains no faces",
        ));
    }

    let mut builder = SceneBuilder::new();
    let mut current: Option<SegmentBuilder> = None;
    let mut material_ids = BTreeMap::new();

    for face in &parsed.faces {
        let key = SegmentKey::from_face(face);
        let should_flush = current.as_ref().is_some_and(|segment| segment.key != key);
        if should_flush && let Some(segment) = current.take() {
            flush_segment(ctx, &mut builder, &mut material_ids, library, segment)?;
        }
        if current.is_none() {
            current = Some(SegmentBuilder::new(key));
        }
        let Some(segment) = current.as_mut() else {
            return Err(BaoziError::parse(
                ctx.source().to_string(),
                None,
                "OBJ mesh segment could not be created",
            ));
        };
        segment.push_face(ctx, &parsed, face)?;
    }

    if let Some(segment) = current {
        flush_segment(ctx, &mut builder, &mut material_ids, library, segment)?;
    }

    builder.finish()
}

fn flush_segment(
    ctx: &mut ImportContext<'_>,
    builder: &mut SceneBuilder,
    material_ids: &mut BTreeMap<String, MaterialId>,
    library: &MaterialLibrary,
    segment: SegmentBuilder,
) -> Result<()> {
    let next_meshes = builder
        .mesh_count()
        .checked_add(1)
        .ok_or(BaoziError::LimitExceeded {
            limit: "max_meshes",
        })?;
    if next_meshes > ctx.limits().max_meshes {
        return Err(BaoziError::LimitExceeded {
            limit: "max_meshes",
        });
    }

    let material = segment
        .key
        .material
        .clone()
        .map(|name| resolve_material(ctx, builder, material_ids, library, &name));
    segment.warn_partial_attributes(ctx);
    let mesh = segment.finish(material);
    let name = mesh.name.clone();
    let mesh_id = builder.add_mesh(mesh);
    builder.add_child_node(
        builder.root(),
        Node {
            name,
            meshes: vec![mesh_id],
            ..Node::default()
        },
    )?;
    Ok(())
}

fn resolve_material(
    ctx: &mut ImportContext<'_>,
    builder: &mut SceneBuilder,
    material_ids: &mut BTreeMap<String, MaterialId>,
    library: &MaterialLibrary,
    name: &str,
) -> MaterialId {
    if let Some(id) = material_ids.get(name) {
        return *id;
    }

    let material = if let Some(parsed) = library.materials.get(name) {
        material_from_parsed(builder, parsed)
    } else {
        mtl::push_warning(
            ctx,
            ctx.source().to_string(),
            None,
            "obj.material_missing",
            format!("OBJ material '{name}' was referenced but not defined"),
        );
        Material {
            name: Some(name.to_owned()),
            ..Material::default()
        }
    };
    let id = builder.add_material(material);
    material_ids.insert(name.to_owned(), id);
    id
}

fn material_from_parsed(builder: &mut SceneBuilder, parsed: &ParsedMaterial) -> Material {
    let mut material = Material {
        name: Some(parsed.name.clone()),
        shading_model: ShadingModel::Phong,
        base_color: parsed.base_color,
        emissive: parsed.emissive,
        alpha_mode: parsed.alpha_mode,
        metadata: parsed.metadata.clone(),
        ..Material::default()
    };

    if parsed.base_color.a < 1.0 {
        material.alpha_mode = AlphaMode::Blend;
    }

    if let Some(texture) = &parsed.diffuse_texture {
        let texture_id = builder.add_texture(Texture {
            name: Some(texture.uri.clone()),
            source: TextureSource::External {
                uri: texture.uri.clone(),
            },
            sampler: Default::default(),
            metadata: Default::default(),
        });
        material.texture_slots.push(TextureSlot {
            texture: texture_id,
            role: TextureRole::Diffuse,
            color_space: ColorSpace::Srgb,
            uv_set: 0,
            scale: 1.0,
            transform: Default::default(),
            source_key: Some(texture.source_key.to_owned()),
        });
    }

    material
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
