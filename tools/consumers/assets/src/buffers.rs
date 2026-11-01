use std::collections::{BTreeMap, HashMap};

use baozi::{
    Aabb, AnimationInterpolation, AnimationTarget, AttributeDomain, AttributeElement,
    AttributeSemanticRef, AuthoredTransform, BasePoseWorldTransforms, CancellationToken, Mat4,
    MaterialFactors, MetadataOwnerRef, MetadataValueRef, MorphDeltaSemantic, PrimitiveId,
    SourceObjectLocatorRef, SpatialQueryError, SpatialQueryLimits, TextureRole, TextureTransform,
    ValidatedScene,
    postprocess::{PrimitiveVertex, triangle_vertices},
};

use crate::Result;

/// This application deliberately chooses F64 positions/normals/tangents and UV set zero.
/// F32 inputs widen exactly. Reuse requires the same source vertex and selected field bits.
/// It does not weld distinct source vertices, truncate influences, or decode images.
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

// This key is local to one draw. Include the complete source identity and only the selected
// application fields; Corner/Face seams must survive even when floating-point values compare equal.
#[derive(PartialEq, Eq, Hash)]
struct VertexKey {
    primitive: PrimitiveId,
    source_vertex: u32,
    position: [u64; 3],
    normal: Option<[u64; 3]>,
    tangent: Option<[u64; 4]>,
    uv: Option<[u64; 2]>,
}

impl VertexKey {
    fn new(source: PrimitiveVertex, vertex: &Vertex) -> Self {
        Self {
            primitive: source.primitive,
            source_vertex: source.index,
            position: vertex.position.map(f64::to_bits),
            normal: vertex.normal.map(|v| v.map(f64::to_bits)),
            tangent: vertex.tangent.map(|v| v.map(f64::to_bits)),
            uv: vertex.uv.map(|v| v.map(f64::to_bits)),
        }
    }
}

struct IndexedVertices {
    lookup: HashMap<VertexKey, u32>,
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    representative_corners: Vec<usize>,
    influences: Option<Influences>,
}

impl IndexedVertices {
    fn new(corner_count: usize, has_influences: bool) -> Result<Self> {
        let mut indices = Vec::new();
        indices.try_reserve_exact(corner_count)?;
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
        Ok(Self {
            lookup: HashMap::new(),
            vertices: Vec::new(),
            indices,
            representative_corners: Vec::new(),
            influences,
        })
    }

    fn push(
        &mut self,
        source: PrimitiveVertex,
        vertex: Vertex,
        influences: Option<(&[u32], &[f32])>,
    ) -> Result<()> {
        self.indices.try_reserve(1)?;
        let key = VertexKey::new(source, &vertex);
        let index = if let Some(&index) = self.lookup.get(&key) {
            index
        } else {
            let index = vertex_index(self.vertices.len())?;
            self.lookup.try_reserve(1)?;
            self.vertices.try_reserve(1)?;
            self.representative_corners.try_reserve(1)?;
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
            self.representative_corners.push(self.indices.len());
            // Output order comes only from first Corner encounter, never map iteration.
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
    /// Copies this application's selected layout from an already processed triangle Scene.
    /// All output owns its data. Unsupported topology or ambiguous field domains return an error;
    /// this step does not generate fields or change coordinates.
    pub fn from_scene(scene: &ValidatedScene) -> Result<Self> {
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
                let mut indexed = IndexedVertices::new(
                    usize::try_from(primitive.corner_count())?,
                    primitive.influences().is_some(),
                )?;
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
                    };
                    // Influences and morphs are Vertex-domain values: source identity makes
                    // them equivalent for every matching key, so copy only its representative.
                    indexed.push(
                        tuple.source_vertex(),
                        vertex,
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
                        for &corner in &indexed.representative_corners {
                            let tuple = tuples
                                .vertex(corner)
                                .ok_or("missing representative corner")?;
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
                    material: primitive.material().map(|m| materials[&m.id()]),
                    vertices: indexed.vertices,
                    indices: indexed.indices,
                    influences: indexed.influences,
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
            uv: Some([0.0, 0.0]),
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
            actual.uv.map(|v| v.map(f64::to_bits)),
            expected.uv.map(|v| v.map(f64::to_bits))
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
        let mut uv = base.clone();
        uv.uv = Some([0.5, 0.0]);
        let mut signed_zero_uv = base.clone();
        signed_zero_uv.uv = Some([-0.0, 0.0]);

        // These selected tuples cover Corner/Face variation that the built-in glTF profile
        // cannot author for Tangent. The source identity is deliberately unchanged at corner 3.
        for seam in [
            position,
            normal,
            signed_zero_normal,
            tangent,
            tangent_w,
            uv,
            signed_zero_uv,
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
                let mut packed = IndexedVertices::new(corners.len(), false)?;
                for (&index, vertex) in [0, 1, 2, 0, 2, 1].iter().zip(&corners) {
                    packed.push(PrimitiveVertex { primitive, index }, vertex.clone(), None)?;
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
        corners[8].uv = Some([-0.0, 0.0]);
        let mut packed = IndexedVertices::new(corners.len(), true)?;
        for (&index, vertex) in sources.iter().zip(&corners) {
            let source = usize::try_from(index)?;
            packed.push(
                PrimitiveVertex { primitive, index },
                vertex.clone(),
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
        for ((&index, &source), expected) in packed.indices.iter().zip(&sources).zip(&corners) {
            let index = usize::try_from(index)?;
            let source = usize::try_from(source)?;
            assert_vertex_bits(&packed.vertices[index], expected);
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
    fn allocation_and_index_overflow_return_structured_failures() -> Result<()> {
        let error = IndexedVertices::new(usize::MAX, false)
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
        let empty = IndexedVertices::new(0, true)?;
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
