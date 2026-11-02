use std::{collections::HashMap, fmt, num::NonZeroU32};

use baozi::{
    Aabb, AnimationExtrapolation, AnimationInterpolation, AnimationTarget, AnimationTimeDomain,
    AttributeData, AttributeDomain, AttributeElement, AttributeRef, AttributeSemanticRef,
    AuthoredTransform, BasePoseWorldTransforms, CancellationToken, ColorInterpretation, Mat4,
    MaterialAssignment, MaterialFactors, MetadataOwnerRef, MetadataValueRef, MorphDeltaSemantic,
    PrimitiveId, PrimitiveRef, SamplerState, SceneProfileOwner, SkinMatrixSpaces,
    SourceObjectLocatorRef, SpatialQueryError, SpatialQueryLimits, TangentBasisRef,
    TangentEncodingRef, TextureBlendOperation, TextureChannelSwizzle, TextureRole, TextureStrength,
    TextureTransform, TopologyKind, UvOrigin, ValidatedScene,
    postprocess::{PrimitiveVertex, list_vertices},
};

use crate::Result;

/// This application chooses F64 positions/normals/tangents and linear COLOR_0.
/// UVs live in parallel per-draw streams, with every retained set participating in indexing.
/// RGB colors gain alpha one; absent colors remain absent and RGBA alpha stays authored.
/// F32 inputs widen exactly. Reuse requires the same source vertex and selected field bits.
/// It does not weld distinct source vertices, truncate influences, or decode images.
#[derive(Debug, Clone, PartialEq)]
pub struct Vertex {
    pub position: [f64; 3],
    pub normal: Option<[f64; 3]>,
    pub tangent: Option<[f64; 4]>,
    pub color: Option<[f64; 4]>,
}

#[derive(Debug, Default)]
pub struct Influences {
    pub offsets: Vec<usize>,
    pub joints: Vec<u32>,
    pub weights: Vec<f32>,
}

// This key is local to one draw. Include the complete source identity and only the selected
// application fields; Corner/Face seams must survive even when floating-point values compare equal.
#[derive(PartialEq, Eq, Hash)]
struct VertexKey {
    primitive: PrimitiveId,
    source_vertex: u32,
    position: [u64; 3],
    normal: Option<[u64; 3]>,
    tangent: Option<[u64; 4]>,
    uvs: Vec<[u64; 2]>,
    color: Option<[u64; 4]>,
}

impl VertexKey {
    fn new(source: PrimitiveVertex, vertex: &Vertex, uvs: &[[f64; 2]]) -> Result<Self> {
        let mut uv_bits = Vec::new();
        uv_bits.try_reserve_exact(uvs.len())?;
        uv_bits.extend(uvs.iter().map(|uv| uv.map(f64::to_bits)));
        Ok(Self {
            primitive: source.primitive,
            source_vertex: source.index,
            position: vertex.position.map(f64::to_bits),
            normal: vertex.normal.map(|v| v.map(f64::to_bits)),
            tangent: vertex.tangent.map(|v| v.map(f64::to_bits)),
            uvs: uv_bits,
            color: vertex.color.map(|v| v.map(f64::to_bits)),
        })
    }
}

struct IndexedVertices {
    lookup: HashMap<VertexKey, u32>,
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    // Expanded output tuple ordinals, which can differ from source Corner indices.
    representative_corners: Vec<usize>,
    influences: Option<Influences>,
    uv_streams: Vec<UvStream>,
}

impl IndexedVertices {
    fn new(
        tuple_count: usize,
        has_influences: bool,
        uv_sets: &[(u32, AttributeDomain)],
    ) -> Result<Self> {
        let mut indices = Vec::new();
        indices.try_reserve_exact(tuple_count)?;
        let influences = if has_influences {
            let mut offsets = Vec::new();
            offsets.try_reserve_exact(1)?;
            offsets.push(0);
            Some(Influences {
                offsets,
                ..Influences::default()
            })
        } else {
            None
        };
        let mut uv_streams = Vec::new();
        uv_streams.try_reserve_exact(uv_sets.len())?;
        for &(set, domain) in uv_sets {
            uv_streams.push(UvStream {
                set,
                domain,
                values: Vec::new(),
            });
        }
        Ok(Self {
            lookup: HashMap::new(),
            vertices: Vec::new(),
            indices,
            representative_corners: Vec::new(),
            influences,
            uv_streams,
        })
    }

    fn push(
        &mut self,
        source: PrimitiveVertex,
        vertex: Vertex,
        uvs: &[[f64; 2]],
        influences: Option<(&[u32], &[f32])>,
    ) -> Result<()> {
        self.indices.try_reserve(1)?;
        if uvs.len() != self.uv_streams.len() {
            return Err("UV tuple width does not match the selected streams".into());
        }
        let key = VertexKey::new(source, &vertex, uvs)?;
        let index = if let Some(&index) = self.lookup.get(&key) {
            index
        } else {
            let index = vertex_index(self.vertices.len())?;
            self.lookup.try_reserve(1)?;
            self.vertices.try_reserve(1)?;
            self.representative_corners.try_reserve(1)?;
            for stream in &mut self.uv_streams {
                stream.values.try_reserve(1)?;
            }
            if let Some(out) = &mut self.influences {
                let (joints, weights) = influences.ok_or("missing influence row")?;
                out.offsets.try_reserve(1)?;
                out.joints.try_reserve(joints.len())?;
                out.weights.try_reserve(weights.len())?;
                out.joints.extend_from_slice(joints);
                out.weights.extend_from_slice(weights);
                out.offsets.push(out.joints.len());
            }
            self.vertices.push(vertex);
            for (stream, &uv) in self.uv_streams.iter_mut().zip(uvs) {
                stream.values.push(uv);
            }
            self.representative_corners.push(self.indices.len());
            // Output order comes only from first output tuple encounter, never map iteration.
            self.lookup.insert(key, index);
            index
        };
        self.indices.push(index);
        Ok(())
    }
}

fn vertex_index(vertex_count: usize) -> Result<u32> {
    Ok(u32::try_from(vertex_count)?)
}

#[derive(Debug)]
pub struct MorphStream {
    pub target: usize,
    pub semantic: MorphDeltaSemantic,
    pub values: Vec<[f32; 3]>,
}

/// A retained UV set in output vertex order. No set, domain, transform, or origin is inferred.
#[derive(Debug)]
pub struct UvStream {
    pub set: u32,
    pub domain: AttributeDomain,
    pub values: Vec<[f64; 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UvExtractionErrorKind {
    Missing,
    AmbiguousDomain,
    UnsupportedFormat,
}

/// A failed retained binding/UV closure. Binding ordinals are local to the draw's material.
/// An unreferenced ambiguous/unsupported stream has no binding ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UvExtractionError {
    pub mesh: usize,
    pub primitive: usize,
    pub binding: Option<usize>,
    pub uv_set: u32,
    pub kind: UvExtractionErrorKind,
}

impl fmt::Display for UvExtractionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "mesh {} primitive {} binding {:?} UV set {}: {:?}",
            self.mesh, self.primitive, self.binding, self.uv_set, self.kind
        )
    }
}

impl std::error::Error for UvExtractionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TangentBasis {
    Unknown,
    MikkTspace,
    TriangleUvDerivative,
    Namespaced { name: String, version: NonZeroU32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TangentEncoding {
    Unknown,
    SignInW,
    Namespaced { name: String, version: NonZeroU32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TangentConventions {
    pub domain: AttributeDomain,
    pub basis: TangentBasis,
    pub encoding: TangentEncoding,
}

#[derive(Debug)]
pub struct DrawBuffer {
    pub mesh: usize,
    pub primitive: usize,
    /// Preserves authored absence independently from the effective material's application index.
    pub material: MaterialAssignment<usize>,
    /// Explicit list topology of these buffers, independent of the source primitive topology.
    pub topology: TopologyKind,
    pub vertices: Vec<Vertex>,
    /// Ordered by set; all values use exactly the same vertex indices as `vertices`.
    pub uv_streams: Vec<UvStream>,
    pub tangent_conventions: Option<TangentConventions>,
    pub indices: Vec<u32>,
    pub influences: Option<Influences>,
    pub morphs: Vec<MorphStream>,
}

impl DrawBuffer {
    /// Looks up exactly the requested set, without substituting another UV stream.
    pub fn uv_stream(&self, set: u32) -> Option<&UvStream> {
        self.uv_streams
            .binary_search_by_key(&set, |stream| stream.set)
            .ok()
            .map(|index| &self.uv_streams[index])
    }
}

#[derive(Debug)]
pub struct Node {
    pub name: Option<String>,
    pub transform: AuthoredTransform,
    pub children: Vec<usize>,
}

#[derive(Debug)]
pub struct Instance {
    pub node: usize,
    pub mesh: usize,
    pub skin: Option<usize>,
    /// An authored instance override; absence resolves to the mesh defaults at evaluation time.
    pub morph_weights: Option<Vec<f32>>,
}

#[derive(Debug)]
pub struct Skin {
    pub joints: Vec<usize>,
    pub inverse_binds: Vec<Mat4>,
    pub matrix_spaces: SkinMatrixSpaces,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelTarget {
    Translation(usize),
    Rotation(usize),
    Scale(usize),
    MorphWeights(usize),
}

#[derive(Debug)]
pub struct Channel {
    pub target: ChannelTarget,
    pub interpolation: AnimationInterpolation,
    pub pre_extrapolation: AnimationExtrapolation,
    pub post_extrapolation: AnimationExtrapolation,
    pub width: usize,
    pub times: Vec<f64>,
    pub values: Vec<f32>,
    pub in_tangents: Option<Vec<f32>>,
    pub out_tangents: Option<Vec<f32>>,
}

#[derive(Debug)]
pub struct Clip {
    pub name: Option<String>,
    pub duration: f64,
    pub time_domain: AnimationTimeDomain,
    pub channels: Vec<Channel>,
}

#[derive(Debug)]
pub struct Binding {
    pub role: TextureRole,
    pub texture: usize,
    pub uv_set: u16,
    pub channel_mapping: TextureChannelSwizzle,
    pub color_interpretation: ColorInterpretation,
    pub transform: TextureTransform,
    pub strength: TextureStrength,
    pub blend: TextureBlendOperation,
    pub source_key: Option<String>,
}

#[derive(Debug)]
pub struct Texture {
    pub name: Option<String>,
    pub image: usize,
    pub sampler: usize,
}

#[derive(Debug)]
pub struct Sampler {
    pub name: Option<String>,
    pub state: SamplerState,
}

#[derive(Debug)]
pub struct Image {
    pub name: Option<String>,
    pub encoded_bytes: Vec<u8>,
    pub media_type: Option<String>,
}

#[derive(Debug)]
pub struct Material {
    pub name: Option<String>,
    pub shading_model: baozi::ShadingModel,
    pub alpha_mode: baozi::AlphaMode,
    pub double_sided: bool,
    pub factors: MaterialFactors,
    pub bindings: Vec<Binding>,
}

/// The application's owned subset of glTF source-object JSON metadata.
/// Locator text remains qualified by the format-owned schema and version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceJsonMetadata {
    pub resource: String,
    pub format: String,
    pub locator_schema: String,
    pub locator_schema_version: u32,
    pub locator: String,
    pub key: String,
    pub media_type: String,
    pub payload: Vec<u8>,
}

/// An opt-in owned snapshot in the Scene's current coordinates, using authored base poses.
///
/// Bounds include every Position vertex of each mesh instance, including unreferenced vertices;
/// uninstanced meshes do not contribute. This does not sample animation, apply skinning, or
/// evaluate morphs. Bound skins and effective nonzero morph weights return a typed error.
/// The transform table and bounds remain usable after the source Scene is dropped.
#[derive(Debug)]
pub struct BasePoseSpatialSnapshot {
    pub world_transforms: BasePoseWorldTransforms,
    pub scene_bounds: Option<Aabb>,
}

impl BasePoseSpatialSnapshot {
    /// Queries world transforms, then instance-aware bounds, without changing the Scene.
    ///
    /// `limits` applies independently to each of these two library operations. Cancellation is
    /// forwarded to both operations. This optional query is separate from buffer extraction so
    /// animated and deformed assets can still use [`ApplicationAsset::from_scene`].
    pub fn from_scene(
        scene: &ValidatedScene,
        limits: SpatialQueryLimits,
        cancellation: Option<&CancellationToken>,
    ) -> std::result::Result<Self, SpatialQueryError> {
        let world_transforms = scene.base_pose_world_transforms(limits, cancellation)?;
        let scene_bounds = world_transforms.scene_bounds(scene, limits, cancellation)?;
        Ok(Self {
            world_transforms,
            scene_bounds,
        })
    }
}

#[derive(Debug)]
pub struct ApplicationAsset {
    pub mesh_count: usize,
    /// One default-weight vector per mesh, independent of instance overrides and animation.
    pub mesh_morph_weights: Vec<Vec<f32>>,
    pub roots: Vec<usize>,
    pub nodes: Vec<Node>,
    pub draws: Vec<DrawBuffer>,
    pub instances: Vec<Instance>,
    pub skins: Vec<Skin>,
    pub animations: Vec<Clip>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub samplers: Vec<Sampler>,
    pub images: Vec<Image>,
    pub uv_origin: UvOrigin,
    pub source_json_metadata: Vec<SourceJsonMetadata>,
}

impl ApplicationAsset {
    /// Copies this application's selected layout as Points, LineList, or TriangleList draws.
    /// Strips, fans, and loops expand without changing the Scene; polygons need triangulation.
    /// All output owns its data. Unsupported topology or ambiguous field domains return an error.
    /// This direct extraction does not generate fields or change coordinates and is independent
    /// of the triangle editing, space conversion, and normal generation in [`crate::process_file`].
    pub fn from_scene(scene: &ValidatedScene) -> Result<Self> {
        // Public IDs stay opaque. The application creates its own dense indices exactly once,
        // after the final Scene edit, for references that must outlive that Scene.
        let nodes = dense_indices(scene.nodes().map(|n| n.id()))?;
        let meshes = dense_indices(scene.meshes().map(|m| m.id()))?;
        let skins = dense_indices(scene.skins().map(|s| s.id()))?;
        let instances = dense_indices(scene.mesh_instances().map(|i| i.id()))?;
        let material_indices = dense_indices(scene.materials().map(|m| m.id()))?;
        let image_indices = dense_indices(scene.images().map(|i| i.id()))?;
        let texture_indices = dense_indices(scene.textures().map(|t| t.id()))?;
        let sampler_indices = dense_indices(scene.samplers().map(|s| s.id()))?;
        let mut materials = Vec::new();
        materials.try_reserve_exact(scene.material_count())?;
        for material in scene.materials() {
            let mut bindings = Vec::new();
            bindings.try_reserve_exact(material.binding_count())?;
            for binding in material.bindings() {
                bindings.push(Binding {
                    role: binding.role(),
                    texture: texture_indices[&binding.texture().id()],
                    uv_set: binding.uv_set(),
                    channel_mapping: binding.channel_mapping(),
                    color_interpretation: binding.color_interpretation(),
                    transform: binding.transform(),
                    strength: binding.strength(),
                    blend: binding.blend(),
                    source_key: copy_name(binding.source_key())?,
                });
            }
            materials.push(Material {
                name: copy_name(material.name())?,
                shading_model: material.shading_model(),
                alpha_mode: material.alpha_mode(),
                double_sided: material.double_sided(),
                factors: material.factors(),
                bindings,
            });
        }
        let mut textures = Vec::new();
        textures.try_reserve_exact(scene.texture_count())?;
        for texture in scene.textures() {
            textures.push(Texture {
                name: None,
                image: image_indices[&texture.image().id()],
                sampler: sampler_indices[&texture.sampler().id()],
            });
        }
        let mut samplers = Vec::new();
        samplers.try_reserve_exact(scene.sampler_count())?;
        for sampler in scene.samplers() {
            samplers.push(Sampler {
                name: None,
                state: sampler.state(),
            });
        }
        let mut images = Vec::new();
        images.try_reserve_exact(scene.image_count())?;
        for image in scene.images() {
            images.push(Image {
                name: None,
                encoded_bytes: copy_slice(image.encoded_bytes())?,
                media_type: copy_name(image.media_type())?,
            });
        }
        let mut draws = Vec::new();
        for (mesh_index, mesh) in scene.meshes().enumerate() {
            for (primitive_index, primitive) in mesh.primitives().enumerate() {
                let tuples = list_vertices(primitive)?;
                let material = primitive
                    .material_assignment()
                    .map(|m| material_indices[&m.id()]);
                let bindings = material
                    .effective()
                    .map_or(&[][..], |index| materials[index].bindings.as_slice());
                let uv_sets = select_uv_sets(primitive, bindings, mesh_index, primitive_index)?;
                let mut indexed =
                    IndexedVertices::new(tuples.len(), primitive.influences().is_some(), &uv_sets)?;
                // The app selects one normal, tangent, each UV set and COLOR_0 field. Ambiguous
                // simultaneous domains require an explicit application policy, never priority.
                let normal = select_domain(
                    primitive
                        .attributes()
                        .filter(|a| a.semantic() == AttributeSemanticRef::Normal)
                        .map(|a| a.domain()),
                )?;
                let tangent = select_domain(
                    primitive
                        .attributes()
                        .filter(|a| a.semantic() == AttributeSemanticRef::Tangent)
                        .map(|a| a.domain()),
                )?;
                let color = select_domain(
                    primitive
                        .attributes()
                        .filter(|a| a.semantic() == AttributeSemanticRef::Color(0))
                        .map(|a| a.domain()),
                )?;
                let tangent_conventions = tangent
                    .map(|domain| {
                        copy_tangent_conventions(
                            primitive
                                .attribute(AttributeSemanticRef::Tangent, domain)
                                .ok_or("missing selected tangent descriptor")?,
                        )
                    })
                    .transpose()?
                    .flatten();
                let mut uv_values = Vec::new();
                uv_values.try_reserve_exact(uv_sets.len())?;
                for tuple in tuples.vertices() {
                    let position = tuple
                        .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
                        .ok_or("application requires vertex positions")?;
                    let vertex = Vertex {
                        position: numeric(position.element())?,
                        normal: normal
                            .map(|domain| {
                                numeric(
                                    tuple
                                        .attribute(AttributeSemanticRef::Normal, domain)
                                        .expect("selected field exists")
                                        .element(),
                                )
                            })
                            .transpose()?,
                        tangent: tangent
                            .map(|domain| {
                                numeric(
                                    tuple
                                        .attribute(AttributeSemanticRef::Tangent, domain)
                                        .expect("selected field exists")
                                        .element(),
                                )
                            })
                            .transpose()?,
                        color: color
                            .map(|domain| {
                                numeric_color(
                                    tuple
                                        .attribute(AttributeSemanticRef::Color(0), domain)
                                        .expect("selected field exists")
                                        .element(),
                                )
                            })
                            .transpose()?,
                    };
                    uv_values.clear();
                    for &(set, domain) in &uv_sets {
                        let error = |kind| UvExtractionError {
                            mesh: mesh_index,
                            primitive: primitive_index,
                            binding: bindings
                                .iter()
                                .position(|binding| u32::from(binding.uv_set) == set),
                            uv_set: set,
                            kind,
                        };
                        let field = tuple
                            .attribute(AttributeSemanticRef::Texcoord(set), domain)
                            .ok_or_else(|| error(UvExtractionErrorKind::Missing))?;
                        let value = numeric(field.element())
                            .map_err(|_| error(UvExtractionErrorKind::UnsupportedFormat))?;
                        uv_values.push(value);
                    }
                    // Influences and morphs are Vertex-domain values: source identity makes
                    // them equivalent for every matching key, so copy only its representative.
                    indexed.push(
                        tuple.source_vertex(),
                        vertex,
                        &uv_values,
                        tuple
                            .influences()
                            .map(|row| (row.joint_indices(), row.weights())),
                    )?;
                }
                let mut morphs = Vec::new();
                for (target_index, target) in primitive.morph_targets().enumerate() {
                    for delta in target.deltas() {
                        let mut values = Vec::new();
                        values.try_reserve_exact(indexed.vertices.len())?;
                        for &tuple_index in &indexed.representative_corners {
                            let tuple = tuples
                                .vertex(tuple_index)
                                .ok_or("missing representative output tuple")?;
                            let v = tuple
                                .morph_delta(target.id(), delta.semantic())?
                                .ok_or("missing morph stream")?;
                            values.push([v.x, v.y, v.z]);
                        }
                        morphs.try_reserve(1)?;
                        morphs.push(MorphStream {
                            target: target_index,
                            semantic: delta.semantic(),
                            values,
                        });
                    }
                }
                draws.try_reserve(1)?;
                draws.push(DrawBuffer {
                    mesh: mesh_index,
                    primitive: primitive_index,
                    material,
                    topology: tuples.topology(),
                    vertices: indexed.vertices,
                    uv_streams: indexed.uv_streams,
                    tangent_conventions,
                    indices: indexed.indices,
                    influences: indexed.influences,
                    morphs,
                });
            }
        }
        let mut animations = Vec::new();
        animations.try_reserve_exact(scene.animation_clip_count())?;
        for clip in scene.animation_clips() {
            let mut channels = Vec::new();
            channels.try_reserve_exact(clip.channel_count())?;
            for c in clip.channels() {
                let target = match c.target() {
                    AnimationTarget::NodeTranslation(id) => ChannelTarget::Translation(nodes[&id]),
                    AnimationTarget::NodeRotation(id) => ChannelTarget::Rotation(nodes[&id]),
                    AnimationTarget::NodeScale(id) => ChannelTarget::Scale(nodes[&id]),
                    AnimationTarget::MeshInstanceMorphWeights(id) => {
                        ChannelTarget::MorphWeights(instances[&id])
                    }
                };
                channels.push(Channel {
                    target,
                    interpolation: c.interpolation(),
                    pre_extrapolation: c.pre_extrapolation(),
                    post_extrapolation: c.post_extrapolation(),
                    width: c.value_width(),
                    times: copy_slice(c.key_times())?,
                    values: copy_slice(c.values())?,
                    in_tangents: c.in_tangents().map(copy_slice).transpose()?,
                    out_tangents: c.out_tangents().map(copy_slice).transpose()?,
                });
            }
            animations.push(Clip {
                name: copy_name(clip.name())?,
                duration: clip.duration(),
                time_domain: clip.time_domain(),
                channels,
            });
        }
        let mut mesh_morph_weights = Vec::new();
        mesh_morph_weights.try_reserve_exact(scene.mesh_count())?;
        for mesh in scene.meshes() {
            mesh_morph_weights.push(copy_slice(mesh.default_morph_weights())?);
        }
        let mut copied_nodes = Vec::new();
        copied_nodes.try_reserve_exact(nodes.len())?;
        for node in scene.nodes() {
            let mut children = Vec::new();
            children.try_reserve_exact(node.children().len())?;
            for child in node.children() {
                children.push(nodes[&child.id()]);
            }
            copied_nodes.push(Node {
                name: copy_name(node.name())?,
                transform: node.authored_transform(),
                children,
            });
        }
        let mut roots = Vec::new();
        roots.try_reserve_exact(scene.roots().len())?;
        for node in scene.roots() {
            roots.push(nodes[&node.id()]);
        }
        let mut copied_instances = Vec::new();
        copied_instances.try_reserve_exact(instances.len())?;
        for instance in scene.mesh_instances() {
            copied_instances.push(Instance {
                node: nodes[&instance.node().id()],
                mesh: meshes[&instance.mesh().id()],
                skin: instance.skin().map(|skin| skins[&skin.id()]),
                morph_weights: instance.morph_weights().map(copy_slice).transpose()?,
            });
        }
        let mut copied_skins = Vec::new();
        copied_skins.try_reserve_exact(skins.len())?;
        for skin in scene.skins() {
            let mut joints = Vec::new();
            joints.try_reserve_exact(skin.joints().len())?;
            for joint in skin.joints() {
                joints.push(nodes[&joint]);
            }
            let mut inverse_binds = Vec::new();
            inverse_binds.try_reserve_exact(skin.inverse_bind_matrices().len())?;
            for matrix in skin.inverse_bind_matrices() {
                inverse_binds.push(matrix);
            }
            copied_skins.push(Skin {
                joints,
                inverse_binds,
                matrix_spaces: skin.matrix_spaces(),
            });
        }
        let mut source_json_metadata = Vec::new();
        for entry in scene.metadata_entries() {
            if let MetadataOwnerRef::Entity(owner) = entry.owner() {
                // Source names belong to these independent owners, never to a display fallback.
                if entry.key() == "org.baozi.gltf:source-name" {
                    let target = match owner.descriptor() {
                        SceneProfileOwner::Image(index) => {
                            Some(&mut images[usize::try_from(index)?].name)
                        }
                        SceneProfileOwner::Texture(index) => {
                            Some(&mut textures[usize::try_from(index)?].name)
                        }
                        SceneProfileOwner::Sampler(index) => {
                            Some(&mut samplers[usize::try_from(index)?].name)
                        }
                        _ => None,
                    };
                    if let Some(target) = target {
                        let MetadataValueRef::String(name) = entry.value() else {
                            return Err("source object name metadata must contain a string".into());
                        };
                        *target = Some(copy_text(name)?);
                    }
                }
                continue;
            }
            let MetadataOwnerRef::SourceObject(source) = entry.owner() else {
                continue;
            };
            let MetadataValueRef::Bytes {
                media_type,
                payload,
            } = entry.value()
            else {
                continue;
            };
            if source.format() != "org.baozi.gltf" || media_type != "application/json" {
                continue;
            }
            let SourceObjectLocatorRef::Namespaced {
                schema,
                schema_version,
                value,
            } = source.locator()
            else {
                continue;
            };
            source_json_metadata.try_reserve(1)?;
            source_json_metadata.push(SourceJsonMetadata {
                resource: copy_text(source.resource().identity())?,
                format: copy_text(source.format())?,
                locator_schema: copy_text(schema)?,
                locator_schema_version: schema_version,
                locator: copy_text(value)?,
                key: copy_text(entry.key())?,
                media_type: copy_text(media_type)?,
                payload: copy_slice(payload)?,
            });
        }
        Ok(Self {
            mesh_count: scene.mesh_count(),
            mesh_morph_weights,
            roots,
            nodes: copied_nodes,
            instances: copied_instances,
            skins: copied_skins,
            animations,
            materials,
            textures,
            samplers,
            images,
            uv_origin: scene.space().uv_origin,
            source_json_metadata,
            draws,
        })
    }

    pub fn vertex_count(&self) -> usize {
        self.draws.iter().map(|d| d.vertices.len()).sum()
    }

    /// Aggregates copied draw vertices in mesh-local coordinates, without applying instances.
    /// Use [`BasePoseSpatialSnapshot`] for authored base-pose world bounds.
    pub fn bounds(&self) -> Option<[[f64; 3]; 2]> {
        let mut vertices = self.draws.iter().flat_map(|d| &d.vertices);
        let first = vertices.next()?.position;
        let mut result = [first, first];
        for v in vertices {
            let [low, high] = &mut result;
            for ((low, high), value) in low.iter_mut().zip(high).zip(v.position) {
                *low = low.min(value);
                *high = high.max(value);
            }
        }
        Some(result)
    }
}

fn dense_indices<I: Eq + std::hash::Hash>(
    ids: impl ExactSizeIterator<Item = I>,
) -> Result<HashMap<I, usize>> {
    let mut indices = HashMap::new();
    indices.try_reserve(ids.len())?;
    for (index, id) in ids.enumerate() {
        indices.insert(id, index);
    }
    Ok(indices)
}

fn copy_slice<T: Copy>(source: &[T]) -> Result<Vec<T>> {
    let mut output = Vec::new();
    output.try_reserve_exact(source.len())?;
    output.extend_from_slice(source);
    Ok(output)
}

fn copy_name(source: Option<&str>) -> Result<Option<String>> {
    source.map(copy_text).transpose()
}

fn copy_text(source: &str) -> Result<String> {
    let mut output = String::new();
    output.try_reserve_exact(source.len())?;
    output.push_str(source);
    Ok(output)
}

fn select_uv_sets(
    primitive: PrimitiveRef<'_>,
    bindings: &[Binding],
    mesh: usize,
    primitive_index: usize,
) -> Result<Vec<(u32, AttributeDomain)>> {
    let mut selected = Vec::new();
    for attribute in primitive.attributes() {
        if let AttributeSemanticRef::Texcoord(set) = attribute.semantic() {
            selected.try_reserve(1)?;
            selected.push((set, attribute.domain()));
        }
    }
    selected.sort_unstable_by_key(|&(set, _)| set);
    validate_uv_sets(&selected, bindings, mesh, primitive_index)?;
    for &(set, domain) in &selected {
        let attribute = primitive
            .attribute(AttributeSemanticRef::Texcoord(set), domain)
            .ok_or("missing selected UV descriptor")?;
        if domain == AttributeDomain::Instance
            || attribute.lanes() != 2
            || !matches!(
                attribute.data(),
                AttributeData::DenseF32 { .. }
                    | AttributeData::DenseF64 { .. }
                    | AttributeData::SparseF32 { .. }
                    | AttributeData::SparseF64 { .. }
            )
        {
            return Err(UvExtractionError {
                mesh,
                primitive: primitive_index,
                binding: bindings
                    .iter()
                    .position(|binding| u32::from(binding.uv_set) == set),
                uv_set: set,
                kind: UvExtractionErrorKind::UnsupportedFormat,
            }
            .into());
        }
    }
    Ok(selected)
}

fn validate_uv_sets(
    selected: &[(u32, AttributeDomain)],
    bindings: &[Binding],
    mesh: usize,
    primitive_index: usize,
) -> Result<()> {
    for (index, binding) in bindings.iter().enumerate() {
        let set = u32::from(binding.uv_set);
        let count = selected
            .iter()
            .filter(|&&(candidate, _)| candidate == set)
            .count();
        if count != 1 {
            return Err(UvExtractionError {
                mesh,
                primitive: primitive_index,
                binding: Some(index),
                uv_set: set,
                kind: if count == 0 {
                    UvExtractionErrorKind::Missing
                } else {
                    UvExtractionErrorKind::AmbiguousDomain
                },
            }
            .into());
        }
    }
    for pair in selected.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(UvExtractionError {
                mesh,
                primitive: primitive_index,
                binding: None,
                uv_set: pair[0].0,
                kind: UvExtractionErrorKind::AmbiguousDomain,
            }
            .into());
        }
    }
    Ok(())
}

fn copy_tangent_conventions(attribute: AttributeRef<'_>) -> Result<Option<TangentConventions>> {
    attribute
        .tangent_conventions()
        .map(|conventions| {
            Ok(TangentConventions {
                domain: attribute.domain(),
                basis: match conventions.basis {
                    TangentBasisRef::Unknown => TangentBasis::Unknown,
                    TangentBasisRef::MikkTspace => TangentBasis::MikkTspace,
                    TangentBasisRef::TriangleUvDerivative => TangentBasis::TriangleUvDerivative,
                    TangentBasisRef::Namespaced { name, version } => TangentBasis::Namespaced {
                        name: copy_text(name)?,
                        version,
                    },
                },
                encoding: match conventions.encoding {
                    TangentEncodingRef::Unknown => TangentEncoding::Unknown,
                    TangentEncodingRef::SignInW => TangentEncoding::SignInW,
                    TangentEncodingRef::Namespaced { name, version } => {
                        TangentEncoding::Namespaced {
                            name: copy_text(name)?,
                            version,
                        }
                    }
                },
            })
        })
        .transpose()
}

fn select_domain(
    mut domains: impl Iterator<Item = AttributeDomain>,
) -> Result<Option<AttributeDomain>> {
    let domain = domains.next();
    if domains.next().is_some() {
        return Err(
            "application layout requires an explicit choice between simultaneous attribute domains"
                .into(),
        );
    }
    Ok(domain)
}

pub(crate) fn numeric<const N: usize>(element: AttributeElement<'_>) -> Result<[f64; N]> {
    let mut result = [0.0; N];
    match element {
        AttributeElement::F32(values) if values.len() == N => {
            for (out, value) in result.iter_mut().zip(values) {
                *out = f64::from(*value);
            }
        }
        AttributeElement::F64(values) if values.len() == N => result.copy_from_slice(values),
        _ => {
            return Err(
                "application field requires floating scalars with the expected width".into(),
            );
        }
    }
    Ok(result)
}

fn numeric_color(element: AttributeElement<'_>) -> Result<[f64; 4]> {
    let mut result = [0.0, 0.0, 0.0, 1.0];
    match element {
        AttributeElement::F32(values) if matches!(values.len(), 3 | 4) => {
            for (out, value) in result.iter_mut().zip(values) {
                *out = f64::from(*value);
            }
        }
        AttributeElement::F64(values) if matches!(values.len(), 3 | 4) => {
            result[..values.len()].copy_from_slice(values);
        }
        _ => return Err("application color requires three or four floating scalars".into()),
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn primitive_id() -> Result<PrimitiveId> {
        let report = baozi::Importer::new()?
            .read_bytes("indexing.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n")?;
        Ok(report
            .scene()
            .meshes()
            .next()
            .ok_or("missing mesh")?
            .primitives()
            .next()
            .ok_or("missing primitive")?
            .id())
    }

    fn vertex() -> Vertex {
        Vertex {
            position: [16_777_217.0, -0.0, f64::from_bits(1)],
            normal: Some([0.0, 0.0, 1.0]),
            tangent: Some([1.0, 0.0, 0.0, 1.0]),
            color: Some([0.0, 0.5, 0.75, 1.0]),
        }
    }

    fn assert_vertex_bits(actual: &Vertex, expected: &Vertex) {
        assert_eq!(
            actual.position.map(f64::to_bits),
            expected.position.map(f64::to_bits)
        );
        assert_eq!(
            actual.normal.map(|v| v.map(f64::to_bits)),
            expected.normal.map(|v| v.map(f64::to_bits))
        );
        assert_eq!(
            actual.tangent.map(|v| v.map(f64::to_bits)),
            expected.tangent.map(|v| v.map(f64::to_bits))
        );
        assert_eq!(
            actual.color.map(|v| v.map(f64::to_bits)),
            expected.color.map(|v| v.map(f64::to_bits))
        );
    }

    #[test]
    fn same_source_reuse_requires_every_selected_f64_field_bit() -> Result<()> {
        let primitive = primitive_id()?;
        let base = vertex();
        let mut position = base.clone();
        position.position[0] = f64::from_bits(base.position[0].to_bits() + 1);
        let mut normal = base.clone();
        normal.normal = Some([0.0, 1.0, 0.0]);
        let mut signed_zero_normal = base.clone();
        signed_zero_normal.normal = Some([-0.0, 0.0, 1.0]);
        let mut tangent = base.clone();
        tangent.tangent = Some([0.0, 1.0, 0.0, 1.0]);
        let mut tangent_w = base.clone();
        tangent_w.tangent = Some([1.0, 0.0, 0.0, -1.0]);
        let mut color = base.clone();
        color.color = Some([0.25, 0.5, 0.75, 1.0]);
        let mut color_alpha = base.clone();
        color_alpha.color = Some([0.0, 0.5, 0.75, 0.5]);
        let mut signed_zero_color = base.clone();
        signed_zero_color.color = Some([-0.0, 0.5, 0.75, 1.0]);
        let mut absent_color = base.clone();
        absent_color.color = None;

        // These selected tuples cover Corner/Face variation that the built-in glTF profile
        // cannot author for Tangent or Color. Source identity stays unchanged at corner 3.
        for seam in [
            position,
            normal,
            signed_zero_normal,
            tangent,
            tangent_w,
            color,
            color_alpha,
            signed_zero_color,
            absent_color,
        ] {
            let corners = [
                base.clone(),
                base.clone(),
                base.clone(),
                seam,
                base.clone(),
                base.clone(),
            ];
            for _ in 0..2 {
                let mut packed = IndexedVertices::new(corners.len(), false, &[])?;
                for (&index, vertex) in [0, 1, 2, 0, 2, 1].iter().zip(&corners) {
                    packed.push(
                        PrimitiveVertex { primitive, index },
                        vertex.clone(),
                        &[],
                        None,
                    )?;
                }
                assert_eq!(packed.indices, [0, 1, 2, 3, 2, 1]);
                assert_eq!(packed.vertices.len(), 4);
                assert_eq!(packed.representative_corners, [0, 1, 2, 3]);
                assert!(packed.influences.is_none());
                for (&index, expected) in packed.indices.iter().zip(&corners) {
                    assert_vertex_bits(&packed.vertices[usize::try_from(index)?], expected);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn five_influence_and_empty_rows_keep_source_identity_and_bits() -> Result<()> {
        let primitive = primitive_id()?;
        let joints: [&[u32]; 3] = [&[1, 0, 1, 0, 1], &[], &[0]];
        let weights: [&[f32]; 3] = [&[-0.0, 0.125, 0.25, 0.375, 0.25], &[], &[1.0]];
        let sources = [0, 1, 2, 0, 2, 1, 0, 2, 1];
        let mut corners = vec![vertex(); sources.len()];
        corners[6].tangent = Some([1.0, 0.0, 0.0, -1.0]);
        let mut uvs = vec![[0.0, 0.0]; sources.len()];
        uvs[8] = [-0.0, 0.0];
        let mut packed =
            IndexedVertices::new(corners.len(), true, &[(0, AttributeDomain::Vertex)])?;
        for ((&index, vertex), &uv) in sources.iter().zip(&corners).zip(&uvs) {
            let source = usize::try_from(index)?;
            packed.push(
                PrimitiveVertex { primitive, index },
                vertex.clone(),
                &[uv],
                Some((joints[source], weights[source])),
            )?;
        }
        assert_eq!(packed.indices, [0, 1, 2, 0, 2, 1, 3, 2, 4]);
        assert_eq!(packed.vertices.len(), 5);
        assert_eq!(packed.representative_corners, [0, 1, 2, 6, 8]);
        let rows = packed.influences.as_ref().ok_or("missing influences")?;
        assert_eq!(rows.offsets, [0, 5, 5, 6, 11, 11]);
        assert_eq!(rows.joints.len(), 11);
        assert_eq!(rows.weights.len(), 11);
        for (((&index, &source), expected), uv) in
            packed.indices.iter().zip(&sources).zip(&corners).zip(&uvs)
        {
            let index = usize::try_from(index)?;
            let source = usize::try_from(source)?;
            assert_vertex_bits(&packed.vertices[index], expected);
            assert_eq!(
                packed.uv_streams[0].values[index].map(f64::to_bits),
                uv.map(f64::to_bits)
            );
            let range = rows.offsets[index]..rows.offsets[index + 1];
            assert_eq!(&rows.joints[range.clone()], joints[source]);
            assert_eq!(
                rows.weights[range]
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                weights[source]
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>()
            );
        }
        Ok(())
    }

    #[test]
    fn every_uv_set_and_f64_lane_bit_participates_in_tuple_reuse() -> Result<()> {
        let primitive = primitive_id()?;
        let sets = [
            (0, AttributeDomain::Vertex),
            (1, AttributeDomain::Corner),
            (70_000, AttributeDomain::Face),
        ];
        let baseline = [[0.0, -0.0], [0.25, 16_777_217.0], [f64::from_bits(1), 0.5]];
        for set in 0..sets.len() {
            for lane in 0..2 {
                let mut changed = baseline;
                changed[set][lane] = f64::from_bits(changed[set][lane].to_bits() ^ 1);
                let mut packed = IndexedVertices::new(3, false, &sets)?;
                for uv in [baseline, changed, baseline] {
                    packed.push(
                        PrimitiveVertex {
                            primitive,
                            index: 0,
                        },
                        vertex(),
                        &uv,
                        None,
                    )?;
                }
                assert_eq!(packed.indices, [0, 1, 0]);
                for (stream, expected) in packed.uv_streams.iter().zip(baseline) {
                    assert_eq!(
                        stream.values[0].map(f64::to_bits),
                        expected.map(f64::to_bits)
                    );
                }
            }
        }
        let mut packed = IndexedVertices::new(2, false, &sets)?;
        let mut signed_zero = baseline;
        signed_zero[1][0] = -0.0;
        let mut positive_zero = baseline;
        positive_zero[1][0] = 0.0;
        for uv in [positive_zero, signed_zero] {
            packed.push(
                PrimitiveVertex {
                    primitive,
                    index: 0,
                },
                vertex(),
                &uv,
                None,
            )?;
        }
        assert_eq!(packed.indices, [0, 1]);
        Ok(())
    }

    #[test]
    fn ambiguous_uv_domains_name_the_draw_binding_and_set() -> Result<()> {
        let selected = [(1, AttributeDomain::Vertex), (1, AttributeDomain::Corner)];
        let binding = Binding {
            role: TextureRole::Normal,
            texture: 0,
            uv_set: 1,
            channel_mapping: TextureChannelSwizzle::RGB,
            color_interpretation: ColorInterpretation::Data,
            transform: TextureTransform::IDENTITY,
            strength: TextureStrength::ONE,
            blend: TextureBlendOperation::Replace,
            source_key: None,
        };
        let error = validate_uv_sets(&selected, &[binding], 2, 3).unwrap_err();
        assert_eq!(
            error.downcast_ref::<UvExtractionError>(),
            Some(&UvExtractionError {
                mesh: 2,
                primitive: 3,
                binding: Some(0),
                uv_set: 1,
                kind: UvExtractionErrorKind::AmbiguousDomain,
            })
        );
        let error = validate_uv_sets(&selected, &[], 2, 3).unwrap_err();
        assert_eq!(
            error.downcast_ref::<UvExtractionError>().unwrap().binding,
            None
        );
        Ok(())
    }

    #[test]
    fn application_numeric_selection_preserves_f64_and_exact_f32_widening() -> Result<()> {
        let wide = [16_777_217.0, -0.0, f64::from_bits(1)];
        assert_eq!(
            numeric::<3>(AttributeElement::F64(&wide))?.map(f64::to_bits),
            wide.map(f64::to_bits)
        );
        let narrow = [-0.0, f32::from_bits(1), 0.125];
        assert_eq!(
            numeric::<3>(AttributeElement::F32(&narrow))?.map(f64::to_bits),
            narrow.map(|v| f64::from(v).to_bits())
        );
        Ok(())
    }

    #[test]
    fn application_color_preserves_rgb_rgba_bits_without_clamping() -> Result<()> {
        let wide_rgb = [16_777_217.0, -0.0, f64::from_bits(1)];
        let wide_rgba = [wide_rgb[0], wide_rgb[1], wide_rgb[2], -0.0];
        assert_eq!(
            numeric_color(AttributeElement::F64(&wide_rgb))?.map(f64::to_bits),
            [wide_rgb[0], wide_rgb[1], wide_rgb[2], 1.0].map(f64::to_bits)
        );
        assert_eq!(
            numeric_color(AttributeElement::F64(&wide_rgba))?.map(f64::to_bits),
            wide_rgba.map(f64::to_bits)
        );
        let narrow_rgb = [-0.0, f32::from_bits(1), 1.25];
        let narrow_rgba = [narrow_rgb[0], narrow_rgb[1], narrow_rgb[2], -0.5];
        assert_eq!(
            numeric_color(AttributeElement::F32(&narrow_rgb))?.map(f64::to_bits),
            [narrow_rgb[0], narrow_rgb[1], narrow_rgb[2], 1.0]
                .map(|value| f64::from(value).to_bits())
        );
        assert_eq!(
            numeric_color(AttributeElement::F32(&narrow_rgba))?.map(f64::to_bits),
            narrow_rgba.map(|value| f64::from(value).to_bits())
        );
        assert!(numeric_color(AttributeElement::F32(&[0.0; 2])).is_err());
        assert!(numeric_color(AttributeElement::F64(&[0.0; 5])).is_err());
        assert!(numeric_color(AttributeElement::U32(&[0; 4])).is_err());
        Ok(())
    }

    #[test]
    fn allocation_and_index_overflow_return_structured_failures() -> Result<()> {
        let error = IndexedVertices::new(usize::MAX, false, &[])
            .err()
            .ok_or("oversized allocation unexpectedly succeeded")?;
        assert!(
            error
                .downcast_ref::<std::collections::TryReserveError>()
                .is_some()
        );
        assert_eq!(vertex_index(usize::try_from(u32::MAX)?)?, u32::MAX);
        if let Ok(overflow) = usize::try_from(u64::from(u32::MAX) + 1) {
            let error = vertex_index(overflow).unwrap_err();
            assert!(error.downcast_ref::<std::num::TryFromIntError>().is_some());
        }
        let empty = IndexedVertices::new(0, true, &[])?;
        assert!(empty.vertices.is_empty());
        assert!(empty.indices.is_empty());
        assert_eq!(empty.influences.ok_or("missing empty CSR")?.offsets, [0]);
        Ok(())
    }

    #[test]
    fn simultaneous_selected_domains_still_require_an_explicit_application_policy() {
        assert!(
            select_domain([AttributeDomain::Vertex, AttributeDomain::Corner].into_iter()).is_err()
        );
        assert!(
            select_domain([AttributeDomain::Corner, AttributeDomain::Face].into_iter()).is_err()
        );
    }
}
