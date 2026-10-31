use std::collections::BTreeMap;

use baozi::{
    AnimationInterpolation, AnimationTarget, AttributeDomain, AttributeElement,
    AttributeSemanticRef, AuthoredTransform, Mat4, MaterialFactors, MetadataOwnerRef,
    MetadataValueRef, MorphDeltaSemantic, SourceObjectLocatorRef, TextureRole, TextureTransform,
    ValidatedScene, postprocess::triangle_vertices,
};

use crate::Result;

/// This application deliberately chooses F64 positions/normals/tangents and UV set zero.
/// F32 inputs widen exactly. It does not weld vertices, truncate influences, or decode images.
#[derive(Debug, Clone, PartialEq)]
pub struct Vertex {
    pub position: [f64; 3],
    pub normal: Option<[f64; 3]>,
    pub tangent: Option<[f64; 4]>,
    pub uv: Option<[f64; 2]>,
}

#[derive(Debug, Default)]
pub struct Influences {
    pub offsets: Vec<usize>,
    pub joints: Vec<u32>,
    pub weights: Vec<f32>,
}

#[derive(Debug)]
pub struct MorphStream {
    pub target: usize,
    pub semantic: MorphDeltaSemantic,
    pub values: Vec<[f32; 3]>,
}

#[derive(Debug)]
pub struct DrawBuffer {
    pub mesh: usize,
    pub primitive: usize,
    pub material: Option<usize>,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub influences: Option<Influences>,
    pub morphs: Vec<MorphStream>,
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
    pub morph_weights: Vec<f32>,
}

#[derive(Debug)]
pub struct Skin {
    pub joints: Vec<usize>,
    pub inverse_binds: Vec<Mat4>,
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
    pub channels: Vec<Channel>,
}

#[derive(Debug)]
pub struct Binding {
    pub role: TextureRole,
    pub image: usize,
    pub uv_set: u16,
    pub transform: TextureTransform,
}

#[derive(Debug)]
pub struct Material {
    pub name: Option<String>,
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

#[derive(Debug)]
pub struct ApplicationAsset {
    pub mesh_count: usize,
    pub roots: Vec<usize>,
    pub nodes: Vec<Node>,
    pub draws: Vec<DrawBuffer>,
    pub instances: Vec<Instance>,
    pub skins: Vec<Skin>,
    pub animations: Vec<Clip>,
    pub materials: Vec<Material>,
    pub images: Vec<Vec<u8>>,
    pub source_json_metadata: Vec<SourceJsonMetadata>,
}

impl ApplicationAsset {
    pub(crate) fn from_scene(scene: &ValidatedScene) -> Result<Self> {
        // Public IDs stay opaque. The application creates its own dense indices exactly once,
        // after the final Scene edit, for references that must outlive that Scene.
        let nodes: BTreeMap<_, _> = scene
            .nodes()
            .enumerate()
            .map(|(i, n)| (n.id(), i))
            .collect();
        let meshes: BTreeMap<_, _> = scene
            .meshes()
            .enumerate()
            .map(|(i, m)| (m.id(), i))
            .collect();
        let skins: BTreeMap<_, _> = scene
            .skins()
            .enumerate()
            .map(|(i, s)| (s.id(), i))
            .collect();
        let instances: BTreeMap<_, _> = scene
            .mesh_instances()
            .enumerate()
            .map(|(i, m)| (m.id(), i))
            .collect();
        let materials: BTreeMap<_, _> = scene
            .materials()
            .enumerate()
            .map(|(i, m)| (m.id(), i))
            .collect();
        let images: BTreeMap<_, _> = scene
            .images()
            .enumerate()
            .map(|(i, m)| (m.id(), i))
            .collect();
        let mut draws = Vec::new();
        for (mesh_index, mesh) in scene.meshes().enumerate() {
            for (primitive_index, primitive) in mesh.primitives().enumerate() {
                let tuples = triangle_vertices(primitive)?;
                let mut vertices = Vec::new();
                vertices.try_reserve_exact(tuples.len())?;
                let mut influences = primitive.influences().map(|_| Influences {
                    offsets: vec![0],
                    ..Influences::default()
                });
                // The app layout selects one normal, tangent and UV0 field. Ambiguous simultaneous
                // domains must be resolved by an explicit application policy, never priority.
                let normal = select_domain(
                    primitive
                        .attributes()
                        .filter(|a| a.semantic() == AttributeSemanticRef::Normal)
                        .map(|a| a.domain()),
                )?;
                let uv = select_domain(
                    primitive
                        .attributes()
                        .filter(|a| a.semantic() == AttributeSemanticRef::Texcoord(0))
                        .map(|a| a.domain()),
                )?;
                let tangent = select_domain(
                    primitive
                        .attributes()
                        .filter(|a| a.semantic() == AttributeSemanticRef::Tangent)
                        .map(|a| a.domain()),
                )?;
                for tuple in tuples.vertices() {
                    let position = tuple
                        .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
                        .ok_or("application requires vertex positions")?;
                    vertices.push(Vertex {
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
                        uv: uv
                            .map(|domain| {
                                numeric(
                                    tuple
                                        .attribute(AttributeSemanticRef::Texcoord(0), domain)
                                        .expect("selected field exists")
                                        .element(),
                                )
                            })
                            .transpose()?,
                    });
                    if let Some(out) = &mut influences {
                        let row = tuple.influences().ok_or("missing influence row")?;
                        out.joints.extend_from_slice(row.joint_indices());
                        out.weights.extend_from_slice(row.weights());
                        out.offsets.push(out.joints.len());
                    }
                }
                let mut morphs = Vec::new();
                for (target_index, target) in primitive.morph_targets().enumerate() {
                    for delta in target.deltas() {
                        let mut values = Vec::new();
                        values.try_reserve_exact(tuples.len())?;
                        for tuple in tuples.vertices() {
                            let v = tuple
                                .morph_delta(target.id(), delta.semantic())?
                                .ok_or("missing morph stream")?;
                            values.push([v.x, v.y, v.z]);
                        }
                        morphs.push(MorphStream {
                            target: target_index,
                            semantic: delta.semantic(),
                            values,
                        });
                    }
                }
                draws.push(DrawBuffer {
                    mesh: mesh_index,
                    primitive: primitive_index,
                    material: primitive.material().map(|m| materials[&m.id()]),
                    vertices,
                    indices: tuples.indices().collect(),
                    influences,
                    morphs,
                });
            }
        }
        let animations = scene
            .animation_clips()
            .map(|clip| {
                let channels = clip
                    .channels()
                    .map(|c| {
                        let target = match c.target() {
                            AnimationTarget::NodeTranslation(id) => {
                                ChannelTarget::Translation(nodes[&id])
                            }
                            AnimationTarget::NodeRotation(id) => {
                                ChannelTarget::Rotation(nodes[&id])
                            }
                            AnimationTarget::NodeScale(id) => ChannelTarget::Scale(nodes[&id]),
                            AnimationTarget::MeshInstanceMorphWeights(id) => {
                                ChannelTarget::MorphWeights(instances[&id])
                            }
                        };
                        Channel {
                            target,
                            interpolation: c.interpolation(),
                            width: c.value_width(),
                            times: c.key_times().to_vec(),
                            values: c.values().to_vec(),
                            in_tangents: c.in_tangents().map(<[f32]>::to_vec),
                            out_tangents: c.out_tangents().map(<[f32]>::to_vec),
                        }
                    })
                    .collect();
                Clip {
                    name: clip.name().map(str::to_owned),
                    duration: clip.duration(),
                    channels,
                }
            })
            .collect();
        Ok(Self {
            mesh_count: scene.mesh_count(),
            roots: scene.roots().map(|n| nodes[&n.id()]).collect(),
            nodes: scene
                .nodes()
                .map(|n| Node {
                    name: n.name().map(str::to_owned),
                    transform: n.authored_transform(),
                    children: n.children().map(|c| nodes[&c.id()]).collect(),
                })
                .collect(),
            instances: scene
                .mesh_instances()
                .map(|i| Instance {
                    node: nodes[&i.node().id()],
                    mesh: meshes[&i.mesh().id()],
                    skin: i.skin().map(|s| skins[&s.id()]),
                    morph_weights: i
                        .morph_weights()
                        .unwrap_or_else(|| i.mesh().default_morph_weights())
                        .to_vec(),
                })
                .collect(),
            skins: scene
                .skins()
                .map(|s| Skin {
                    joints: s.joints().map(|id| nodes[&id]).collect(),
                    inverse_binds: s.inverse_bind_matrices().collect(),
                })
                .collect(),
            animations,
            materials: scene
                .materials()
                .map(|m| Material {
                    name: m.name().map(str::to_owned),
                    factors: m.factors(),
                    bindings: m
                        .bindings()
                        .map(|b| Binding {
                            role: b.role(),
                            image: images[&b.texture().image().id()],
                            uv_set: b.uv_set(),
                            transform: b.transform(),
                        })
                        .collect(),
                })
                .collect(),
            images: scene.images().map(|i| i.encoded_bytes().to_vec()).collect(),
            source_json_metadata: scene
                .metadata_entries()
                .filter_map(|entry| {
                    let MetadataOwnerRef::SourceObject(source) = entry.owner() else {
                        return None;
                    };
                    let MetadataValueRef::Bytes {
                        media_type,
                        payload,
                    } = entry.value()
                    else {
                        return None;
                    };
                    if source.format() != "org.baozi.gltf" || media_type != "application/json" {
                        return None;
                    }
                    let SourceObjectLocatorRef::Namespaced {
                        schema,
                        schema_version,
                        value,
                    } = source.locator()
                    else {
                        return None;
                    };
                    Some(SourceJsonMetadata {
                        resource: source.resource().identity().to_owned(),
                        format: source.format().to_owned(),
                        locator_schema: schema.to_owned(),
                        locator_schema_version: schema_version,
                        locator: value.to_owned(),
                        key: entry.key().to_owned(),
                        media_type: media_type.to_owned(),
                        payload: payload.to_vec(),
                    })
                })
                .collect(),
            draws,
        })
    }

    pub fn vertex_count(&self) -> usize {
        self.draws.iter().map(|d| d.vertices.len()).sum()
    }

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

fn numeric<const N: usize>(element: AttributeElement<'_>) -> Result<[f64; N]> {
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
