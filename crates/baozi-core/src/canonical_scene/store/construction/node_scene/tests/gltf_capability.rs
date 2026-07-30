use std::sync::atomic::AtomicU64;

use baozi_operation::{CancellationToken, PhysicalStorageAccount, StorePreparationFailureClass};

use crate::SceneValidationLimits;
use crate::appearance::{TextureRole, TextureRoleKind};
use crate::attribute::{AttributeDomain, AttributeSemantic};
use crate::canonical_scene::store::construction::node_scene::semantic::{
    NodeSceneAnimationExtrapolation, NodeSceneAnimationInterpolation, NodeSceneAnimationTarget,
    NodeSceneAnimationTimeDomain, NodeSceneDeformationF32Cursor, NodeSceneDeformationMorphPayload,
    NodeSceneGeometryAttributePayload, NodeSceneGeometryAttributeSemantic,
    NodeSceneGeometryF32Cursor, NodeSceneGeometryU32Cursor,
};
use crate::canonical_scene::store::construction::node_scene::snapshot::{
    NodeSceneSnapshot, NodeSceneSnapshotLimits,
};
use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneConstructionTerminal, NodeSceneSuccessParts,
};
use crate::canonical_scene::store::construction::{STORE_DIMENSION, SceneConstructionOperation};
use crate::canonical_scene::store::production::{
    AnimationAuthoringBatch, AnimationAuthoringFailure, AnimationAuthoringSemanticError,
    AnimationChannelAuthoring, AnimationClipAuthoring, AnimationExtrapolationAuthoring,
    AnimationInterpolationAuthoring, AnimationTargetAuthoring, AnimationTimeDomainAuthoring,
    AppearanceAuthoringBatch, AppearanceAuthoringFailure, AppearanceAuthoringSemanticError,
    AuthoringRange, DeformationAuthoringBatch, DeformationAuthoringFailure,
    DeformationAuthoringSemanticError, GeometryAttributeAuthoring,
    GeometryAttributePayloadAuthoring, GeometryAuthoringBatch, GeometryLogicalMeshAuthoring,
    GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    ImageAuthoring, ImageLocalId, InfluenceAuthoring, InfluenceRowsAuthoring, LogicalMeshLocalId,
    LogicalMorphAuthoring, MaterialAuthoring, MaterialBindingAuthoring, MaterialLocalId,
    MeshInstanceDeformationAuthoring, MorphDeltaAuthoring, MorphDeltaPayloadAuthoring,
    MorphTargetDefinitionAuthoring, MorphTargetIdentity, NodeAuthoringBatch, NodeAuthoringNode,
    NodeLocalId, NodeParent, Presence, PrimitiveDeformationAuthoring,
    PrimitiveMorphTargetAuthoring, SamplerAuthoring, SamplerLocalId, SceneAuthoringBatch,
    SkinAuthoring, SkinLocalId, TextureAuthoring, TextureLocalId,
};
use crate::deform::{MorphDeltaSemantic, SkinMatrixSpace, SkinMatrixSpaces};
use crate::entity::{AuthoredTransform, AuthoredTransformKind, EntityName};
use crate::extension::ExtensionMediaType;
use crate::geometry::TopologyKind;
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};
use crate::image::{MipmapFilter, SamplerState, TexelFilter, TextureAddressMode};
use crate::math::{Mat4, Quat, Vec3};

use super::{
    NodeSceneConstructionRejection, assert_released, rich_failure, terminal_failure_class,
    test_account, test_factory, test_limits, test_limits_with_work,
};

pub(in crate::canonical_scene::store) const GLTF_SOURCE_VECTOR_SCHEMA_ID: &str =
    "baozi.selected-gltf-shape-capability-source.v1";

const IMAGE_PNG: [u8; 12] = [
    0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, b'b', b'z', b'0', b'1',
];
const IMAGE_JPEG: [u8; 8] = [0xff, 0xd8, 0xff, 0xe0, b'b', b'z', 0xff, 0xd9];
const TRIANGLE_INDICES: [u32; 3] = [0, 1, 2];
const POSITIONS: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
const NORMALS: [f32; 9] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
const TEXCOORDS: [f32; 6] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
const MORPH_DENSE_A: [f32; 9] = [0.0, 0.0, 0.0, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0];
const MORPH_DENSE_B: [f32; 9] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -0.1, 0.0, 0.0];
const MORPH_SPARSE_A: [f32; 3] = [0.1, 0.0, 0.0];
const MORPH_SPARSE_B: [f32; 3] = [-0.1, 0.0, 0.0];
const MORPH_DEFAULTS: [f32; 2] = [0.0, 0.0];
const MORPH_OVERRIDE: [f32; 2] = [0.25, 0.75];
const KEY_TIMES: [f64; 2] = [0.0, 1.0];
const TRANSLATION_VALUES: [f32; 6] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0];
const ROTATION_VALUES: [f32; 8] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0];
const MORPH_ANIMATION_VALUES: [f32; 4] = [0.0, 0.0, 1.0, 0.5];
const MORPH_TANGENTS: [f32; 4] = [0.0; 4];

#[derive(Clone, Copy, PartialEq, Eq)]
enum GltfCapabilityMutation {
    Baseline,
    NodeTranslation,
    ImagePayload,
    EmptyImagePayload,
    PrimitiveMaterial,
    SkinInverseBind,
    SkinInverseBindCountMismatch,
    InfluenceWeight,
    MorphSparseIndex,
    AnimationValue,
    AnimationTimeOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GltfCapabilityScope {
    ImagePayload,
    Images,
    MixedImageMedia,
    Samplers,
    Textures,
    Appearance,
    Geometry,
    Deformation,
    All,
}

#[derive(Clone, Copy)]
struct GltfSourceImage<'source> {
    bytes: &'source [u8],
    media_type: Presence<&'source ExtensionMediaType>,
}

pub(in crate::canonical_scene::store) struct GltfCapabilityMutationObservation {
    pub(in crate::canonical_scene::store) field_class: &'static str,
    pub(in crate::canonical_scene::store) source_vector: Vec<u8>,
    pub(in crate::canonical_scene::store) snapshot_digest: [u8; 32],
}

pub(in crate::canonical_scene::store) struct GltfCapabilityVectorObservations {
    pub(in crate::canonical_scene::store) source_shape: GltfCapabilitySourceShapeObservation,
    pub(in crate::canonical_scene::store) source_vector: Vec<u8>,
    pub(in crate::canonical_scene::store) baseline_snapshot_digest: [u8; 32],
    pub(in crate::canonical_scene::store) mutations: [GltfCapabilityMutationObservation; 7],
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GltfCapabilitySourceShapeObservation {
    pub(in crate::canonical_scene::store) node_count: usize,
    pub(in crate::canonical_scene::store) root_count: usize,
    pub(in crate::canonical_scene::store) image_count: usize,
    pub(in crate::canonical_scene::store) image_payload_bytes: usize,
    pub(in crate::canonical_scene::store) media_type_bytes: usize,
    pub(in crate::canonical_scene::store) sampler_count: usize,
    pub(in crate::canonical_scene::store) texture_count: usize,
    pub(in crate::canonical_scene::store) material_count: usize,
    pub(in crate::canonical_scene::store) material_binding_count: usize,
    pub(in crate::canonical_scene::store) logical_mesh_count: usize,
    pub(in crate::canonical_scene::store) primitive_count: usize,
    pub(in crate::canonical_scene::store) attribute_count: usize,
    pub(in crate::canonical_scene::store) mesh_instance_count: usize,
    pub(in crate::canonical_scene::store) skin_count: usize,
    pub(in crate::canonical_scene::store) skin_joint_count: usize,
    pub(in crate::canonical_scene::store) morph_definition_count: usize,
    pub(in crate::canonical_scene::store) morph_target_count: usize,
    pub(in crate::canonical_scene::store) morph_delta_count: usize,
    pub(in crate::canonical_scene::store) influence_row_count: usize,
    pub(in crate::canonical_scene::store) influence_count: usize,
    pub(in crate::canonical_scene::store) animation_clip_count: usize,
    pub(in crate::canonical_scene::store) animation_channel_count: usize,
    pub(in crate::canonical_scene::store) animation_key_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct GltfCapabilityCanonicalShapeObservation {
    pub(in crate::canonical_scene::store) node_count: usize,
    pub(in crate::canonical_scene::store) root_count: usize,
    pub(in crate::canonical_scene::store) edge_count: usize,
    pub(in crate::canonical_scene::store) image_count: usize,
    pub(in crate::canonical_scene::store) sampler_count: usize,
    pub(in crate::canonical_scene::store) texture_count: usize,
    pub(in crate::canonical_scene::store) material_count: usize,
    pub(in crate::canonical_scene::store) material_binding_count: usize,
    pub(in crate::canonical_scene::store) logical_mesh_count: usize,
    pub(in crate::canonical_scene::store) primitive_count: usize,
    pub(in crate::canonical_scene::store) attribute_count: usize,
    pub(in crate::canonical_scene::store) mesh_instance_count: usize,
    pub(in crate::canonical_scene::store) skin_count: usize,
    pub(in crate::canonical_scene::store) skin_joint_count: usize,
    pub(in crate::canonical_scene::store) logical_morph_count: usize,
    pub(in crate::canonical_scene::store) morph_definition_count: usize,
    pub(in crate::canonical_scene::store) primitive_deformation_count: usize,
    pub(in crate::canonical_scene::store) morph_target_count: usize,
    pub(in crate::canonical_scene::store) morph_delta_count: usize,
    pub(in crate::canonical_scene::store) influence_row_count: usize,
    pub(in crate::canonical_scene::store) influence_count: usize,
    pub(in crate::canonical_scene::store) animation_clip_count: usize,
    pub(in crate::canonical_scene::store) animation_channel_count: usize,
    pub(in crate::canonical_scene::store) animation_key_count: usize,
}

struct GltfSourceEncoder {
    bytes: Vec<u8>,
}

impl GltfSourceEncoder {
    fn new() -> Self {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(GLTF_SOURCE_VECTOR_SCHEMA_ID.as_bytes());
        bytes.push(0);
        Self { bytes }
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }

    fn tag(&mut self, tag: u8) {
        self.bytes.push(tag);
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn usize(&mut self, value: usize) {
        let value = u64::try_from(value).expect("glTF source vector length must fit u64");
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn f32(&mut self, value: f32) {
        self.bytes.extend_from_slice(&value.to_bits().to_le_bytes());
    }

    fn f64(&mut self, value: f64) {
        self.bytes.extend_from_slice(&value.to_bits().to_le_bytes());
    }

    fn payload(&mut self, value: &[u8]) {
        self.usize(value.len());
        self.bytes.extend_from_slice(value);
    }

    fn utf8(&mut self, value: &str) {
        self.payload(value.as_bytes());
    }

    fn list<T>(&mut self, values: &[T], mut encode: impl FnMut(&mut Self, &T)) {
        self.usize(values.len());
        for value in values {
            encode(self, value);
        }
    }

    fn presence<T>(&mut self, value: Presence<T>, encode: impl FnOnce(&mut Self, T)) {
        match value {
            Presence::Absent => self.u8(0),
            Presence::Present(value) => {
                self.u8(1);
                encode(self, value);
            }
        }
    }
}

fn encode_gltf_source(
    input: SceneAuthoringBatch<'_>,
    source_images: &[GltfSourceImage<'_>],
) -> Vec<u8> {
    fn encode_f32_slice(encoder: &mut GltfSourceEncoder, values: &[f32]) {
        encoder.list(values, |encoder, value| encoder.f32(*value));
    }

    fn encode_f64_slice(encoder: &mut GltfSourceEncoder, values: &[f64]) {
        encoder.list(values, |encoder, value| encoder.f64(*value));
    }

    fn encode_u32_slice(encoder: &mut GltfSourceEncoder, values: &[u32]) {
        encoder.list(values, |encoder, value| encoder.u32(*value));
    }

    fn encode_transform(encoder: &mut GltfSourceEncoder, transform: AuthoredTransform) {
        match transform.kind() {
            AuthoredTransformKind::Matrix => {
                encoder.u8(0);
                let matrix = transform
                    .authored_matrix()
                    .expect("matrix-kind authored transform");
                for value in matrix.cols.into_iter().flatten() {
                    encoder.f32(value);
                }
            }
            AuthoredTransformKind::Trs => {
                encoder.u8(1);
                let trs = transform
                    .authored_trs()
                    .expect("TRS-kind authored transform");
                let translation = trs.translation();
                encoder.f32(translation.x);
                encoder.f32(translation.y);
                encoder.f32(translation.z);
                let rotation = trs.rotation();
                encoder.f32(rotation.x);
                encoder.f32(rotation.y);
                encoder.f32(rotation.z);
                encoder.f32(rotation.w);
                let scale = trs.scale();
                encoder.f32(scale.x);
                encoder.f32(scale.y);
                encoder.f32(scale.z);
            }
        }
    }

    fn encode_texel_filter(value: TexelFilter) -> u8 {
        match value {
            TexelFilter::Nearest => 0,
            TexelFilter::Linear => 1,
        }
    }

    fn encode_mipmap_filter(value: MipmapFilter) -> u8 {
        match value {
            MipmapFilter::None => 0,
            MipmapFilter::Nearest => 1,
            MipmapFilter::Linear => 2,
        }
    }

    fn encode_address_mode(value: TextureAddressMode) -> u8 {
        match value {
            TextureAddressMode::Repeat => 0,
            TextureAddressMode::MirroredRepeat => 1,
            TextureAddressMode::ClampToEdge => 2,
            TextureAddressMode::ClampToBorder => 3,
        }
    }

    fn encode_texture_role(encoder: &mut GltfSourceEncoder, role: &TextureRole) {
        let tag = match role {
            TextureRole::BaseColor => 0,
            TextureRole::Diffuse => 1,
            TextureRole::Specular => 2,
            TextureRole::Metallic => 3,
            TextureRole::Roughness => 4,
            TextureRole::MetallicRoughness => 5,
            TextureRole::Normal => 6,
            TextureRole::Occlusion => 7,
            TextureRole::Emissive => 8,
            TextureRole::Opacity => 9,
            TextureRole::Height => 10,
            TextureRole::Displacement => 11,
            TextureRole::Lightmap => 12,
            TextureRole::Clearcoat => 13,
            TextureRole::ClearcoatRoughness => 14,
            TextureRole::ClearcoatNormal => 15,
            TextureRole::Transmission => 16,
            TextureRole::Thickness => 17,
            TextureRole::SheenColor => 18,
            TextureRole::SheenRoughness => 19,
            TextureRole::Namespaced { .. } => 20,
        };
        encoder.u8(tag);
        if let TextureRole::Namespaced { schema_id } = role {
            encoder.utf8(schema_id.name().as_str());
            encoder.u32(schema_id.version().get());
        }
    }

    fn encode_topology(encoder: &mut GltfSourceEncoder, topology: GeometryTopologyAuthoring<'_>) {
        match topology {
            GeometryTopologyAuthoring::Points => encoder.u8(0),
            GeometryTopologyAuthoring::LineList => encoder.u8(1),
            GeometryTopologyAuthoring::LineStrip => encoder.u8(2),
            GeometryTopologyAuthoring::LineLoop => encoder.u8(3),
            GeometryTopologyAuthoring::TriangleList => encoder.u8(4),
            GeometryTopologyAuthoring::TriangleStrip => encoder.u8(5),
            GeometryTopologyAuthoring::TriangleFan => encoder.u8(6),
            GeometryTopologyAuthoring::Polygons(boundaries) => {
                encoder.u8(7);
                encode_u32_slice(encoder, boundaries);
            }
        }
    }

    fn encode_attribute_semantic(encoder: &mut GltfSourceEncoder, semantic: &AttributeSemantic) {
        match semantic {
            AttributeSemantic::Position => encoder.u8(0),
            AttributeSemantic::Normal => encoder.u8(1),
            AttributeSemantic::Tangent => encoder.u8(2),
            AttributeSemantic::Texcoord(set) => {
                encoder.u8(3);
                encoder.u32(*set);
            }
            AttributeSemantic::Color(set) => {
                encoder.u8(4);
                encoder.u32(*set);
            }
            AttributeSemantic::SmoothingGroup => encoder.u8(5),
            AttributeSemantic::Custom(custom) => {
                encoder.u8(6);
                encoder.utf8(custom.as_str());
            }
        }
    }

    fn encode_attribute_domain(value: AttributeDomain) -> u8 {
        match value {
            AttributeDomain::Vertex => 0,
            AttributeDomain::Corner => 1,
            AttributeDomain::Face => 2,
            AttributeDomain::Instance => 3,
        }
    }

    fn encode_skin_space(value: SkinMatrixSpace) -> u8 {
        match value {
            SkinMatrixSpace::LogicalMeshLocalBind => 0,
            SkinMatrixSpace::JointLocalBind => 1,
            SkinMatrixSpace::JointLocal => 2,
            SkinMatrixSpace::Scene => 3,
            SkinMatrixSpace::MeshInstanceLocal => 4,
        }
    }

    fn encode_matrix(encoder: &mut GltfSourceEncoder, matrix: Mat4) {
        for value in matrix.cols.into_iter().flatten() {
            encoder.f32(value);
        }
    }

    fn encode_matrix_spaces(encoder: &mut GltfSourceEncoder, spaces: SkinMatrixSpaces) {
        encoder.u8(encode_skin_space(spaces.input_position()));
        encoder.u8(encode_skin_space(spaces.inverse_bind_from()));
        encoder.u8(encode_skin_space(spaces.inverse_bind_to()));
        encoder.u8(encode_skin_space(spaces.current_joint_from()));
        encoder.u8(encode_skin_space(spaces.current_joint_to()));
        encoder.u8(encode_skin_space(spaces.output_position()));
        match spaces.correction() {
            Some(correction) => {
                encoder.u8(1);
                encoder.u8(encode_skin_space(correction.from()));
                encoder.u8(encode_skin_space(correction.to()));
                encode_matrix(encoder, correction.matrix());
            }
            None => encoder.u8(0),
        }
    }

    fn encode_morph_semantic(value: MorphDeltaSemantic) -> u8 {
        match value {
            MorphDeltaSemantic::Position => 0,
            MorphDeltaSemantic::Normal => 1,
            MorphDeltaSemantic::Tangent => 2,
        }
    }

    fn encode_animation_target(value: AnimationTargetAuthoring) -> (u8, u32) {
        match value {
            AnimationTargetAuthoring::NodeTranslation(node) => (0, node.index()),
            AnimationTargetAuthoring::NodeRotation(node) => (1, node.index()),
            AnimationTargetAuthoring::NodeScale(node) => (2, node.index()),
            AnimationTargetAuthoring::MeshInstanceMorphWeights(instance) => (3, instance),
        }
    }

    fn encode_interpolation(value: AnimationInterpolationAuthoring) -> u8 {
        match value {
            AnimationInterpolationAuthoring::Step => 0,
            AnimationInterpolationAuthoring::Linear => 1,
            AnimationInterpolationAuthoring::CubicSpline => 2,
        }
    }

    fn encode_extrapolation(value: AnimationExtrapolationAuthoring) -> u8 {
        match value {
            AnimationExtrapolationAuthoring::SourceDefault => 0,
            AnimationExtrapolationAuthoring::Constant => 1,
            AnimationExtrapolationAuthoring::Linear => 2,
            AnimationExtrapolationAuthoring::Repeat => 3,
        }
    }

    let mut encoder = GltfSourceEncoder::new();

    encoder.tag(b'n');
    let nodes = input.nodes();
    encoder.presence(nodes.roots(), |encoder, roots| {
        encoder.list(roots, |encoder, root| encoder.u32(root.index()));
    });
    encoder.list(nodes.nodes(), |encoder, node| {
        encoder.presence(node.parent(), |encoder, parent| {
            encoder.u32(parent.node().index());
            encoder.u32(parent.child_ordinal());
        });
        encode_transform(encoder, node.authored_transform());
    });

    encoder.tag(b'a');
    let appearance = input.appearance();
    assert_eq!(
        appearance.images().len(),
        source_images.len(),
        "glTF image source sidecar must match the authoring batch",
    );
    encoder.list(source_images, |encoder, image| {
        encoder.u8(0);
        encoder.payload(image.bytes);
        encoder.presence(image.media_type, |encoder, media_type| {
            encoder.utf8(media_type.as_str());
        });
    });
    encoder.list(appearance.samplers(), |encoder, sampler| {
        let state = sampler.state();
        encoder.u8(encode_texel_filter(state.minification()));
        encoder.u8(encode_texel_filter(state.magnification()));
        encoder.u8(encode_mipmap_filter(state.mipmap()));
        encoder.u8(encode_address_mode(state.address_u()));
        encoder.u8(encode_address_mode(state.address_v()));
        encoder.u8(encode_address_mode(state.address_w()));
    });
    encoder.list(appearance.textures(), |encoder, texture| {
        encoder.u32(texture.image().index());
        encoder.u32(texture.sampler().index());
    });
    encoder.list(appearance.bindings(), |encoder, binding| {
        encoder.u32(binding.texture().index());
        encode_texture_role(encoder, binding.role());
    });
    encoder.list(appearance.materials(), |encoder, material| {
        let bindings = material.bindings();
        encoder.u32(bindings.offset());
        encoder.u32(bindings.count());
    });

    encoder.tag(b'g');
    let geometry = input.geometry();
    encoder.list(geometry.logical_meshes(), |encoder, mesh| {
        let primitives = mesh.primitives();
        encoder.u32(primitives.offset());
        encoder.u32(primitives.count());
    });
    encoder.list(geometry.primitives(), |encoder, primitive| {
        encode_topology(encoder, primitive.topology());
        encoder.presence(primitive.indices(), |encoder, indices| {
            encode_u32_slice(encoder, indices);
        });
        encoder.presence(primitive.material(), |encoder, material| {
            encoder.u32(material.index());
        });
        let attributes = primitive.attributes();
        encoder.u32(attributes.offset());
        encoder.u32(attributes.count());
    });
    encoder.list(geometry.attributes(), |encoder, attribute| {
        encode_attribute_semantic(encoder, attribute.semantic());
        encoder.u8(encode_attribute_domain(attribute.domain()));
        match attribute.payload() {
            GeometryAttributePayloadAuthoring::DenseF32 {
                lanes,
                source_values,
                source_indices,
            } => {
                encoder.u8(0);
                encoder.u8(lanes);
                encode_f32_slice(encoder, source_values);
                encoder.presence(source_indices, |encoder, indices| {
                    encode_u32_slice(encoder, indices);
                });
            }
            GeometryAttributePayloadAuthoring::DenseU32 { lanes, values } => {
                encoder.u8(1);
                encoder.u8(lanes);
                encode_u32_slice(encoder, values);
            }
            GeometryAttributePayloadAuthoring::SparseF32 {
                lanes,
                logical_count,
                indices,
                values,
                default,
            } => {
                encoder.u8(2);
                encoder.u8(lanes);
                encoder.u32(logical_count);
                encode_u32_slice(encoder, indices);
                encode_f32_slice(encoder, values);
                encode_f32_slice(encoder, default);
            }
            GeometryAttributePayloadAuthoring::SparseU32 {
                lanes,
                logical_count,
                indices,
                values,
                default,
            } => {
                encoder.u8(3);
                encoder.u8(lanes);
                encoder.u32(logical_count);
                encode_u32_slice(encoder, indices);
                encode_u32_slice(encoder, values);
                encode_u32_slice(encoder, default);
            }
        }
    });
    encoder.list(geometry.mesh_instances(), |encoder, instance| {
        encoder.u32(instance.owner().index());
        encoder.u32(instance.logical_mesh().index());
    });

    encoder.tag(b'd');
    let deformation = input.deformation();
    encoder.list(deformation.skins(), |encoder, skin| {
        encoder.presence(skin.name(), |encoder, name| encoder.utf8(name.as_str()));
        encoder.list(skin.joints(), |encoder, joint| encoder.u32(joint.index()));
        encoder.list(skin.inverse_bind_matrices(), |encoder, matrix| {
            encode_matrix(encoder, *matrix);
        });
        encoder.presence(skin.skeleton_root(), |encoder, root| {
            encoder.u32(root.index());
        });
        encode_matrix_spaces(encoder, skin.matrix_spaces());
    });
    encoder.list(
        deformation.morph_target_definitions(),
        |encoder, definition| {
            encoder.u32(definition.identity().value());
            encoder.presence(definition.name(), |encoder, name| {
                encoder.utf8(name.as_str());
            });
        },
    );
    encoder.list(deformation.logical_meshes(), |encoder, logical_morph| {
        let definitions = logical_morph.definitions();
        encoder.u32(definitions.offset());
        encoder.u32(definitions.count());
        encode_f32_slice(encoder, logical_morph.default_weights());
    });
    encoder.list(deformation.primitives(), |encoder, primitive| {
        encoder.presence(primitive.influences(), |encoder, rows| {
            encode_u32_slice(encoder, rows.vertex_offsets());
            encoder.list(rows.influences(), |encoder, influence| {
                encoder.u32(influence.joint());
                encoder.f32(influence.weight());
                encoder.u32(influence.source_order());
            });
        });
        encoder.list(primitive.morph_targets(), |encoder, target| {
            encoder.list(target.deltas(), |encoder, delta| {
                encoder.u8(encode_morph_semantic(delta.semantic()));
                match delta.payload() {
                    MorphDeltaPayloadAuthoring::Dense { values } => {
                        encoder.u8(0);
                        encode_f32_slice(encoder, values);
                    }
                    MorphDeltaPayloadAuthoring::Sparse {
                        logical_count,
                        indices,
                        values,
                    } => {
                        encoder.u8(1);
                        encoder.u32(logical_count);
                        encode_u32_slice(encoder, indices);
                        encode_f32_slice(encoder, values);
                    }
                }
            });
        });
    });
    encoder.list(deformation.mesh_instances(), |encoder, instance| {
        encoder.presence(instance.skin(), |encoder, skin| encoder.u32(skin.index()));
        encoder.presence(instance.morph_weights(), |encoder, weights| {
            encode_f32_slice(encoder, weights);
        });
    });

    encoder.tag(b'k');
    let animations = input.animations();
    encoder.list(animations.clips(), |encoder, clip| {
        encoder.f64(clip.duration());
        match clip.time_domain() {
            AnimationTimeDomainAuthoring::Seconds => encoder.u8(0),
            AnimationTimeDomainAuthoring::Frames { rate } => {
                encoder.u8(1);
                encoder.f64(rate);
            }
            AnimationTimeDomainAuthoring::Ticks { rate } => {
                encoder.u8(2);
                match rate {
                    Some(rate) => {
                        encoder.u8(1);
                        encoder.f64(rate);
                    }
                    None => encoder.u8(0),
                }
            }
        }
        let channels = clip.channels();
        encoder.u32(channels.offset());
        encoder.u32(channels.count());
    });
    encoder.list(animations.channels(), |encoder, channel| {
        let (target, reference) = encode_animation_target(channel.target());
        encoder.u8(target);
        encoder.u32(reference);
        encoder.u8(encode_interpolation(channel.interpolation()));
        encoder.u8(encode_extrapolation(channel.pre_extrapolation()));
        encoder.u8(encode_extrapolation(channel.post_extrapolation()));
        encode_f64_slice(encoder, channel.times());
        encode_f32_slice(encoder, channel.values());
        encoder.presence(channel.in_tangents(), |encoder, tangents| {
            encode_f32_slice(encoder, tangents);
        });
        encoder.presence(channel.out_tangents(), |encoder, tangents| {
            encode_f32_slice(encoder, tangents);
        });
    });

    encoder.finish()
}

fn gltf_source_shape(
    input: SceneAuthoringBatch<'_>,
    source_images: &[GltfSourceImage<'_>],
) -> GltfCapabilitySourceShapeObservation {
    let nodes = input.nodes();
    let appearance = input.appearance();
    let geometry = input.geometry();
    let deformation = input.deformation();
    let animations = input.animations();
    let root_count = match nodes.roots() {
        Presence::Absent => 0,
        Presence::Present(roots) => roots.len(),
    };
    let image_payload_bytes = source_images.iter().map(|image| image.bytes.len()).sum();
    let media_type_bytes = source_images
        .iter()
        .map(|image| match image.media_type {
            Presence::Absent => 0,
            Presence::Present(media_type) => media_type.as_str().len(),
        })
        .sum();
    let skin_joint_count = deformation
        .skins()
        .iter()
        .map(|skin| skin.joints().len())
        .sum();
    let morph_target_count = deformation
        .primitives()
        .iter()
        .map(|primitive| primitive.morph_targets().len())
        .sum();
    let morph_delta_count = deformation
        .primitives()
        .iter()
        .flat_map(|primitive| primitive.morph_targets())
        .map(|target| target.deltas().len())
        .sum();
    let influence_row_count = deformation
        .primitives()
        .iter()
        .map(|primitive| match primitive.influences() {
            Presence::Absent => 0,
            Presence::Present(rows) => rows.vertex_offsets().len().saturating_sub(1),
        })
        .sum();
    let influence_count = deformation
        .primitives()
        .iter()
        .map(|primitive| match primitive.influences() {
            Presence::Absent => 0,
            Presence::Present(rows) => rows.influences().len(),
        })
        .sum();
    let animation_key_count = animations
        .channels()
        .iter()
        .map(|channel| channel.times().len())
        .sum();
    GltfCapabilitySourceShapeObservation {
        node_count: nodes.nodes().len(),
        root_count,
        image_count: appearance.images().len(),
        image_payload_bytes,
        media_type_bytes,
        sampler_count: appearance.samplers().len(),
        texture_count: appearance.textures().len(),
        material_count: appearance.materials().len(),
        material_binding_count: appearance.bindings().len(),
        logical_mesh_count: geometry.logical_meshes().len(),
        primitive_count: geometry.primitives().len(),
        attribute_count: geometry.attributes().len(),
        mesh_instance_count: geometry.mesh_instances().len(),
        skin_count: deformation.skins().len(),
        skin_joint_count,
        morph_definition_count: deformation.morph_target_definitions().len(),
        morph_target_count,
        morph_delta_count,
        influence_row_count,
        influence_count,
        animation_clip_count: animations.clips().len(),
        animation_channel_count: animations.channels().len(),
        animation_key_count,
    }
}

fn with_gltf_capability_input<R>(
    scope: GltfCapabilityScope,
    mutation: GltfCapabilityMutation,
    use_input: impl FnOnce(SceneAuthoringBatch<'_>, &[GltfSourceImage<'_>]) -> R,
) -> R {
    let node_zero_transform = if mutation == GltfCapabilityMutation::NodeTranslation {
        AuthoredTransform::try_trs(
            Vec3::new(0.25, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::new(1.0, 1.0, 1.0),
        )
        .expect("finite glTF node mutation")
    } else {
        AuthoredTransform::identity_trs()
    };
    let roots = [NodeLocalId::new(0), NodeLocalId::new(1)];
    let nodes = [
        NodeAuthoringNode::new(Presence::Absent, node_zero_transform),
        NodeAuthoringNode::new(Presence::Absent, AuthoredTransform::identity_trs()),
        NodeAuthoringNode::new(
            Presence::Present(NodeParent::new(NodeLocalId::new(1), 0)),
            AuthoredTransform::identity_trs(),
        ),
    ];
    let node_batch = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);

    let png_media_type = ExtensionMediaType::new("image/png").expect("PNG media type");
    let jpeg_media_type = ExtensionMediaType::new("image/jpeg").expect("JPEG media type");
    let mut png_payload = IMAGE_PNG;
    if mutation == GltfCapabilityMutation::ImagePayload {
        png_payload[10] ^= 1;
    }
    let png_payload: &[u8] = if mutation == GltfCapabilityMutation::EmptyImagePayload {
        &[]
    } else {
        &png_payload
    };
    let images = [
        ImageAuthoring::encoded(png_payload, Presence::Present(&png_media_type)),
        ImageAuthoring::encoded(&IMAGE_JPEG, Presence::Present(&jpeg_media_type)),
    ];
    let images_without_media = [
        ImageAuthoring::encoded(png_payload, Presence::Absent),
        ImageAuthoring::encoded(&IMAGE_JPEG, Presence::Absent),
    ];
    let mixed_media_images = [
        ImageAuthoring::encoded(png_payload, Presence::Present(&png_media_type)),
        ImageAuthoring::encoded(&IMAGE_JPEG, Presence::Absent),
        ImageAuthoring::encoded(&IMAGE_PNG, Presence::Present(&png_media_type)),
    ];
    let source_images = [
        GltfSourceImage {
            bytes: png_payload,
            media_type: Presence::Present(&png_media_type),
        },
        GltfSourceImage {
            bytes: &IMAGE_JPEG,
            media_type: Presence::Present(&jpeg_media_type),
        },
    ];
    let source_images_without_media = [
        GltfSourceImage {
            bytes: png_payload,
            media_type: Presence::Absent,
        },
        GltfSourceImage {
            bytes: &IMAGE_JPEG,
            media_type: Presence::Absent,
        },
    ];
    let mixed_source_images = [
        GltfSourceImage {
            bytes: png_payload,
            media_type: Presence::Present(&png_media_type),
        },
        GltfSourceImage {
            bytes: &IMAGE_JPEG,
            media_type: Presence::Absent,
        },
        GltfSourceImage {
            bytes: &IMAGE_PNG,
            media_type: Presence::Present(&png_media_type),
        },
    ];
    let samplers = [
        SamplerAuthoring::new(SamplerState::CANONICAL_DEFAULT),
        SamplerAuthoring::new(SamplerState::new(
            TexelFilter::Nearest,
            TexelFilter::Linear,
            MipmapFilter::None,
            TextureAddressMode::ClampToEdge,
            TextureAddressMode::MirroredRepeat,
            TextureAddressMode::Repeat,
        )),
    ];
    let textures = [
        TextureAuthoring::new(ImageLocalId::new(0), SamplerLocalId::new(0)),
        TextureAuthoring::new(ImageLocalId::new(0), SamplerLocalId::new(1)),
        TextureAuthoring::new(ImageLocalId::new(1), SamplerLocalId::new(0)),
    ];
    let roles = [
        TextureRole::BaseColor,
        TextureRole::MetallicRoughness,
        TextureRole::Normal,
    ];
    let bindings = [
        MaterialBindingAuthoring::new(TextureLocalId::new(0), &roles[0]),
        MaterialBindingAuthoring::new(TextureLocalId::new(2), &roles[1]),
        MaterialBindingAuthoring::new(TextureLocalId::new(1), &roles[2]),
    ];
    let materials = [
        MaterialAuthoring::new(AuthoringRange::new(0, 2)),
        MaterialAuthoring::new(AuthoringRange::new(2, 1)),
    ];
    let appearance =
        AppearanceAuthoringBatch::new(&images, &samplers, &textures, &bindings, &materials);
    let image_only_appearance = AppearanceAuthoringBatch::new(&images, &[], &[], &[], &[]);
    let image_payload_only_appearance =
        AppearanceAuthoringBatch::new(&images_without_media, &[], &[], &[], &[]);
    let mixed_media_appearance =
        AppearanceAuthoringBatch::new(&mixed_media_images, &[], &[], &[], &[]);
    let sampler_only_appearance = AppearanceAuthoringBatch::new(&[], &samplers, &[], &[], &[]);
    let texture_only_appearance =
        AppearanceAuthoringBatch::new(&images, &samplers, &textures, &[], &[]);

    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &POSITIONS,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Vertex,
            3,
            &NORMALS,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Texcoord(0),
            AttributeDomain::Vertex,
            2,
            &TEXCOORDS,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &POSITIONS,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Vertex,
            3,
            &NORMALS,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Texcoord(0),
            AttributeDomain::Vertex,
            2,
            &TEXCOORDS,
            Presence::Absent,
        ),
    ];
    let primitive_one_material = if mutation == GltfCapabilityMutation::PrimitiveMaterial {
        MaterialLocalId::new(0)
    } else {
        MaterialLocalId::new(1)
    };
    let primitives = [
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&TRIANGLE_INDICES),
            Presence::Present(MaterialLocalId::new(0)),
            AuthoringRange::new(0, 3),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&TRIANGLE_INDICES),
            Presence::Present(primitive_one_material),
            AuthoringRange::new(3, 3),
        ),
    ];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 2))];
    let geometry_instances = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(0),
    )];
    let geometry = GeometryAuthoringBatch::new(
        &logical_meshes,
        &primitives,
        &attributes,
        &geometry_instances,
    );

    let skin_name = EntityName::try_new("Armature").expect("skin name");
    let morph_names = [
        EntityName::try_new("Smile").expect("morph name"),
        EntityName::try_new("Blink").expect("morph name"),
    ];
    let skin_joints = [NodeLocalId::new(1), NodeLocalId::new(2)];
    let mut inverse_bind_matrices = [
        Mat4::IDENTITY,
        Mat4 {
            cols: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, -1.0, 0.0, 1.0],
            ],
        },
    ];
    if mutation == GltfCapabilityMutation::SkinInverseBind {
        inverse_bind_matrices[1].cols[3][1] = -1.25;
    }
    let inverse_bind_matrices: &[Mat4] =
        if mutation == GltfCapabilityMutation::SkinInverseBindCountMismatch {
            &inverse_bind_matrices[..1]
        } else {
            &inverse_bind_matrices
        };
    let skins = [SkinAuthoring::new(
        Presence::Present(&skin_name),
        &skin_joints,
        inverse_bind_matrices,
        Presence::Present(NodeLocalId::new(1)),
        SkinMatrixSpaces::canonical(),
    )];
    let morph_definitions = [
        MorphTargetDefinitionAuthoring::new(
            MorphTargetIdentity::new(0),
            Presence::Present(&morph_names[0]),
        ),
        MorphTargetDefinitionAuthoring::new(
            MorphTargetIdentity::new(1),
            Presence::Present(&morph_names[1]),
        ),
    ];
    let logical_morphs = [LogicalMorphAuthoring::new(
        AuthoringRange::new(0, 2),
        &MORPH_DEFAULTS,
    )];

    let mut primitive_zero_influences = [
        InfluenceAuthoring::new(0, 0.40, 0),
        InfluenceAuthoring::new(1, 0.30, 1),
        InfluenceAuthoring::new(0, 0.15, 2),
        InfluenceAuthoring::new(1, 0.10, 3),
        InfluenceAuthoring::new(0, 0.05, 4),
        InfluenceAuthoring::new(0, 0.25, 0),
        InfluenceAuthoring::new(1, 0.75, 1),
        InfluenceAuthoring::new(1, 1.00, 0),
    ];
    if mutation == GltfCapabilityMutation::InfluenceWeight {
        primitive_zero_influences[0] = InfluenceAuthoring::new(0, 0.45, 0);
    }
    let primitive_zero_offsets = [0, 5, 7, 8];
    let primitive_one_offsets = [0, 1, 2, 3];
    let primitive_one_influences = [
        InfluenceAuthoring::new(0, 1.0, 0),
        InfluenceAuthoring::new(1, 1.0, 0),
        InfluenceAuthoring::new(1, 1.0, 0),
    ];
    let primitive_zero_rows =
        InfluenceRowsAuthoring::new(&primitive_zero_offsets, &primitive_zero_influences);
    let primitive_one_rows =
        InfluenceRowsAuthoring::new(&primitive_one_offsets, &primitive_one_influences);

    let sparse_zero_indices = if mutation == GltfCapabilityMutation::MorphSparseIndex {
        [2]
    } else {
        [1]
    };
    let sparse_one_indices = [2];
    let primitive_zero_target_zero_deltas = [MorphDeltaAuthoring::new(
        MorphDeltaSemantic::Position,
        MorphDeltaPayloadAuthoring::Dense {
            values: &MORPH_DENSE_A,
        },
    )];
    let primitive_zero_target_one_deltas = [MorphDeltaAuthoring::new(
        MorphDeltaSemantic::Position,
        MorphDeltaPayloadAuthoring::Sparse {
            logical_count: 3,
            indices: &sparse_zero_indices,
            values: &MORPH_SPARSE_A,
        },
    )];
    let primitive_one_target_zero_deltas = [MorphDeltaAuthoring::new(
        MorphDeltaSemantic::Position,
        MorphDeltaPayloadAuthoring::Dense {
            values: &MORPH_DENSE_B,
        },
    )];
    let primitive_one_target_one_deltas = [MorphDeltaAuthoring::new(
        MorphDeltaSemantic::Position,
        MorphDeltaPayloadAuthoring::Sparse {
            logical_count: 3,
            indices: &sparse_one_indices,
            values: &MORPH_SPARSE_B,
        },
    )];
    let primitive_zero_targets = [
        PrimitiveMorphTargetAuthoring::new(&primitive_zero_target_zero_deltas),
        PrimitiveMorphTargetAuthoring::new(&primitive_zero_target_one_deltas),
    ];
    let primitive_one_targets = [
        PrimitiveMorphTargetAuthoring::new(&primitive_one_target_zero_deltas),
        PrimitiveMorphTargetAuthoring::new(&primitive_one_target_one_deltas),
    ];
    let primitive_deformations = [
        PrimitiveDeformationAuthoring::new(
            Presence::Present(primitive_zero_rows),
            &primitive_zero_targets,
        ),
        PrimitiveDeformationAuthoring::new(
            Presence::Present(primitive_one_rows),
            &primitive_one_targets,
        ),
    ];
    let mesh_instance_deformations = [MeshInstanceDeformationAuthoring::new(
        Presence::Present(SkinLocalId::new(0)),
        Presence::Present(&MORPH_OVERRIDE),
    )];
    let deformation = DeformationAuthoringBatch::new(
        &skins,
        &morph_definitions,
        &logical_morphs,
        &primitive_deformations,
        &mesh_instance_deformations,
    );
    let empty_logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let empty_primitive_deformations = [
        PrimitiveDeformationAuthoring::new(Presence::Absent, &[]),
        PrimitiveDeformationAuthoring::new(Presence::Absent, &[]),
    ];
    let empty_mesh_instance_deformations = [MeshInstanceDeformationAuthoring::new(
        Presence::Absent,
        Presence::Absent,
    )];
    let aligned_empty_deformation = DeformationAuthoringBatch::new(
        &[],
        &[],
        &empty_logical_morphs,
        &empty_primitive_deformations,
        &empty_mesh_instance_deformations,
    );

    let mut morph_animation_values = MORPH_ANIMATION_VALUES;
    if mutation == GltfCapabilityMutation::AnimationValue {
        morph_animation_values[3] = 0.75;
    }
    let invalid_key_times = [0.0, 0.0];
    let key_times = if mutation == GltfCapabilityMutation::AnimationTimeOrder {
        &invalid_key_times
    } else {
        &KEY_TIMES
    };
    let animation_channels = [
        AnimationChannelAuthoring::new(
            AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
            AnimationInterpolationAuthoring::Linear,
            AnimationExtrapolationAuthoring::SourceDefault,
            AnimationExtrapolationAuthoring::SourceDefault,
            key_times,
            &TRANSLATION_VALUES,
            Presence::Absent,
            Presence::Absent,
        ),
        AnimationChannelAuthoring::new(
            AnimationTargetAuthoring::NodeRotation(NodeLocalId::new(2)),
            AnimationInterpolationAuthoring::Step,
            AnimationExtrapolationAuthoring::SourceDefault,
            AnimationExtrapolationAuthoring::SourceDefault,
            key_times,
            &ROTATION_VALUES,
            Presence::Absent,
            Presence::Absent,
        ),
        AnimationChannelAuthoring::new(
            AnimationTargetAuthoring::MeshInstanceMorphWeights(0),
            AnimationInterpolationAuthoring::CubicSpline,
            AnimationExtrapolationAuthoring::SourceDefault,
            AnimationExtrapolationAuthoring::SourceDefault,
            key_times,
            &morph_animation_values,
            Presence::Present(&MORPH_TANGENTS),
            Presence::Present(&MORPH_TANGENTS),
        ),
    ];
    let animation_clips = [AnimationClipAuthoring::new(
        1.0,
        AnimationTimeDomainAuthoring::Seconds,
        AuthoringRange::new(0, 3),
    )];
    let animations = AnimationAuthoringBatch::new(&animation_clips, &animation_channels);
    let empty_source_images: &[GltfSourceImage<'_>] = &[];

    let (appearance, geometry, deformation, animations, selected_source_images) = match scope {
        GltfCapabilityScope::ImagePayload => (
            image_payload_only_appearance,
            GeometryAuthoringBatch::empty(),
            DeformationAuthoringBatch::empty(),
            AnimationAuthoringBatch::empty(),
            &source_images_without_media[..],
        ),
        GltfCapabilityScope::Images => (
            image_only_appearance,
            GeometryAuthoringBatch::empty(),
            DeformationAuthoringBatch::empty(),
            AnimationAuthoringBatch::empty(),
            &source_images[..],
        ),
        GltfCapabilityScope::MixedImageMedia => (
            mixed_media_appearance,
            GeometryAuthoringBatch::empty(),
            DeformationAuthoringBatch::empty(),
            AnimationAuthoringBatch::empty(),
            &mixed_source_images[..],
        ),
        GltfCapabilityScope::Samplers => (
            sampler_only_appearance,
            GeometryAuthoringBatch::empty(),
            DeformationAuthoringBatch::empty(),
            AnimationAuthoringBatch::empty(),
            empty_source_images,
        ),
        GltfCapabilityScope::Textures => (
            texture_only_appearance,
            GeometryAuthoringBatch::empty(),
            DeformationAuthoringBatch::empty(),
            AnimationAuthoringBatch::empty(),
            &source_images[..],
        ),
        GltfCapabilityScope::Appearance => (
            appearance,
            GeometryAuthoringBatch::empty(),
            DeformationAuthoringBatch::empty(),
            AnimationAuthoringBatch::empty(),
            &source_images[..],
        ),
        GltfCapabilityScope::Geometry => (
            appearance,
            geometry,
            aligned_empty_deformation,
            AnimationAuthoringBatch::empty(),
            &source_images[..],
        ),
        GltfCapabilityScope::Deformation => (
            appearance,
            geometry,
            deformation,
            AnimationAuthoringBatch::empty(),
            &source_images[..],
        ),
        GltfCapabilityScope::All => (
            appearance,
            geometry,
            deformation,
            animations,
            &source_images[..],
        ),
    };
    use_input(
        SceneAuthoringBatch::new(node_batch, appearance, geometry, deformation, animations),
        selected_source_images,
    )
}

fn with_gltf_scene_input<R>(
    scope: GltfCapabilityScope,
    mutation: GltfCapabilityMutation,
    use_input: impl FnOnce(SceneAuthoringBatch<'_>) -> R,
) -> R {
    with_gltf_capability_input(scope, mutation, |input, _| use_input(input))
}

fn build_gltf_scene_with_generation_source(
    account: &PhysicalStorageAccount,
    scope: GltfCapabilityScope,
    mutation: GltfCapabilityMutation,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> NodeSceneSuccessParts {
    with_gltf_scene_input(scope, mutation, |input| {
        build_gltf_scene_input_with_generation_source(account, input, scope, reserve_generation)
    })
}

fn build_gltf_scene_input_with_generation_source(
    account: &PhysicalStorageAccount,
    input: SceneAuthoringBatch<'_>,
    scope: GltfCapabilityScope,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> NodeSceneSuccessParts {
    let prepared = SceneConstructionOperation::with_account_for_scene_input(
        account,
        test_limits(),
        input,
        SceneValidationLimits::default(),
    )
    .expect("selected normalized glTF shape admission")
    .try_validate()
    .unwrap_or_else(|_| panic!("selected normalized glTF shape must pass central validation"))
    .prepare_with_generation_source(reserve_generation)
    .expect("selected normalized glTF shape publication");
    match prepared.handoff() {
        NodeSceneConstructionTerminal::Success(parts) => parts,
        NodeSceneConstructionTerminal::RichFailure(_) => {
            panic!("{scope:?} glTF shape published rich failure")
        }
        NodeSceneConstructionTerminal::MinimalFallback(_) => {
            panic!("{scope:?} glTF shape published minimal fallback")
        }
    }
}

fn build_gltf_scene(
    account: &PhysicalStorageAccount,
    mutation: GltfCapabilityMutation,
) -> NodeSceneSuccessParts {
    build_gltf_scene_with_generation_source(
        account,
        GltfCapabilityScope::All,
        mutation,
        SceneGenerationId::reserve,
    )
}

pub(in crate::canonical_scene::store) fn build_gltf_capability_scene_for_probe(
    account: &PhysicalStorageAccount,
    generation_source: &AtomicU64,
) -> NodeSceneSuccessParts {
    build_gltf_scene_with_generation_source(
        account,
        GltfCapabilityScope::All,
        GltfCapabilityMutation::Baseline,
        || SceneGenerationId::reserve_from(generation_source),
    )
}

pub(in crate::canonical_scene::store) fn collect_gltf_capability_vectors_for_probe()
-> GltfCapabilityVectorObservations {
    fn observe(
        account: &PhysicalStorageAccount,
        mutation: GltfCapabilityMutation,
    ) -> (GltfCapabilitySourceShapeObservation, Vec<u8>, [u8; 32]) {
        with_gltf_capability_input(GltfCapabilityScope::All, mutation, |input, images| {
            let source_shape = gltf_source_shape(input, images);
            let source_vector = encode_gltf_source(input, images);
            let parts = build_gltf_scene_input_with_generation_source(
                account,
                input,
                GltfCapabilityScope::All,
                SceneGenerationId::reserve,
            );
            let snapshot = parts
                .scene
                .capture_snapshot(snapshot_limits(0, 0), None)
                .expect("complete glTF capability mutation digest");
            drop(parts);
            (source_shape, source_vector, snapshot.digest)
        })
    }

    let factory = test_factory();
    let account = test_account(&factory, 512);
    let (source_shape, source_vector, baseline_snapshot_digest) =
        observe(&account, GltfCapabilityMutation::Baseline);
    let mutations = [
        (
            "node_transform_float_bits",
            GltfCapabilityMutation::NodeTranslation,
        ),
        ("image_payload_bytes", GltfCapabilityMutation::ImagePayload),
        (
            "primitive_material_reference",
            GltfCapabilityMutation::PrimitiveMaterial,
        ),
        (
            "skin_inverse_bind_float_bits",
            GltfCapabilityMutation::SkinInverseBind,
        ),
        (
            "influence_weight_float_bits",
            GltfCapabilityMutation::InfluenceWeight,
        ),
        (
            "morph_sparse_index",
            GltfCapabilityMutation::MorphSparseIndex,
        ),
        (
            "animation_value_float_bits",
            GltfCapabilityMutation::AnimationValue,
        ),
    ]
    .map(|(field_class, mutation)| {
        let (_, mutation_source_vector, snapshot_digest) = observe(&account, mutation);
        GltfCapabilityMutationObservation {
            field_class,
            source_vector: mutation_source_vector,
            snapshot_digest,
        }
    });

    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);

    GltfCapabilityVectorObservations {
        source_shape,
        source_vector,
        baseline_snapshot_digest,
        mutations,
    }
}

fn assert_geometry_f32(mut values: NodeSceneGeometryF32Cursor<'_>, expected: &[f32]) {
    assert_eq!(values.declared_count(), expected.len());
    for expected in expected {
        assert_eq!(
            values.next().expect("glTF geometry f32 value"),
            Some(*expected)
        );
    }
    assert_eq!(values.next().expect("glTF geometry f32 exhaustion"), None);
    values.finish().expect("complete glTF geometry f32 payload");
}

fn assert_geometry_u32(mut values: NodeSceneGeometryU32Cursor<'_>, expected: &[u32]) {
    assert_eq!(values.declared_count(), expected.len());
    for expected in expected {
        assert_eq!(
            values.next().expect("glTF geometry u32 value"),
            Some(*expected)
        );
    }
    assert_eq!(values.next().expect("glTF geometry u32 exhaustion"), None);
    values.finish().expect("complete glTF geometry u32 payload");
}

fn assert_deformation_f32(mut values: NodeSceneDeformationF32Cursor<'_>, expected: &[f32]) {
    assert_eq!(values.declared_count(), expected.len());
    for expected in expected {
        assert_eq!(
            values.next().expect("glTF deformation f32 value"),
            Some(*expected),
        );
    }
    assert_eq!(
        values.next().expect("glTF deformation f32 exhaustion"),
        None,
    );
    values
        .finish()
        .expect("complete glTF deformation f32 payload");
}

pub(in crate::canonical_scene::store) fn inspect_gltf_capability_semantics_for_probe(
    parts: &NodeSceneSuccessParts,
) -> GltfCapabilityCanonicalShapeObservation {
    let view = parts
        .scene
        .semantic_view()
        .expect("published glTF Scene semantic view");
    let node_count = view.node_count();
    let root_count = view.root_count();
    let edge_count = view.edge_count();
    let image_count = view.image_count();
    let sampler_count = view.sampler_count();
    let texture_count = view.texture_count();
    let material_count = view.material_count();
    let material_binding_count = view.material_binding_count();
    let logical_mesh_count = view.logical_mesh_count();
    let primitive_count = view.primitive_count();
    let attribute_count = view.attribute_count();
    let mesh_instance_count = view.mesh_instance_count();
    let skin_count = view.skin_count();
    let logical_morph_count = view.logical_morph_count();
    let morph_definition_count = view.morph_definition_count();
    let primitive_deformation_count = view.primitive_deformation_count();
    let morph_target_count = view.morph_target_count();
    let morph_delta_count = view.morph_delta_count();
    let animation_clip_count = view.animation_clip_count();
    let animation_channel_count = view.animation_channel_count();
    assert_eq!(
        (
            node_count,
            root_count,
            edge_count,
            image_count,
            sampler_count,
            texture_count,
            material_count,
            material_binding_count,
        ),
        (3, 2, 1, 2, 2, 3, 2, 3),
    );
    assert_eq!(
        (
            logical_mesh_count,
            primitive_count,
            attribute_count,
            mesh_instance_count,
            skin_count,
            logical_morph_count,
            morph_definition_count,
            primitive_deformation_count,
            morph_target_count,
            morph_delta_count,
            animation_clip_count,
            animation_channel_count,
        ),
        (1, 2, 6, 1, 1, 1, 2, 2, 4, 4, 1, 3),
    );

    let mut cursor = view.into_cursor().expect("glTF Scene semantic cursor");
    let mut roots = cursor.take_roots().expect("glTF Scene roots");
    assert_eq!(roots.next().expect("first glTF root"), Some(0));
    assert_eq!(roots.next().expect("second glTF root"), Some(1));
    assert_eq!(roots.next().expect("glTF root exhaustion"), None);
    roots.finish().expect("complete glTF root traversal");

    let expected_children: [&[usize]; 3] = [&[], &[2], &[]];
    for (source_index, expected_children) in expected_children.into_iter().enumerate() {
        let node = cursor
            .next_node()
            .expect("glTF node read")
            .expect("declared glTF node");
        assert_eq!(node.source_index(), source_index);
        assert!(
            node.authored_transform()
                .has_same_authored_bits(AuthoredTransform::identity_trs())
        );
        let mut children = node.into_children();
        assert_eq!(children.declared_count(), expected_children.len());
        for expected in expected_children {
            assert_eq!(children.next().expect("glTF child read"), Some(*expected));
        }
        assert_eq!(children.next().expect("glTF child exhaustion"), None);
        children.finish().expect("complete glTF child traversal");
    }
    assert!(cursor.next_node().expect("glTF node exhaustion").is_none());

    {
        let appearance = cursor.appearance().expect("glTF appearance cursor");
        let expected_images = [
            (&IMAGE_PNG[..], Some("image/png")),
            (&IMAGE_JPEG[..], Some("image/jpeg")),
        ];
        for (source_index, (payload, media_type)) in expected_images.into_iter().enumerate() {
            let image = appearance
                .next_image()
                .expect("glTF image read")
                .expect("declared glTF image");
            assert_eq!(image.source_index(), source_index);
            assert_eq!(image.encoded_payload(), payload);
            assert_eq!(image.media_type(), media_type);
        }
        assert!(
            appearance
                .next_image()
                .expect("glTF image exhaustion")
                .is_none()
        );

        let expected_samplers = [
            SamplerState::CANONICAL_DEFAULT,
            SamplerState::new(
                TexelFilter::Nearest,
                TexelFilter::Linear,
                MipmapFilter::None,
                TextureAddressMode::ClampToEdge,
                TextureAddressMode::MirroredRepeat,
                TextureAddressMode::Repeat,
            ),
        ];
        for (source_index, expected) in expected_samplers.into_iter().enumerate() {
            let sampler = appearance
                .next_sampler()
                .expect("glTF sampler read")
                .expect("declared glTF sampler");
            assert_eq!(sampler.source_index(), source_index);
            assert_eq!(sampler.state(), expected);
        }
        assert!(
            appearance
                .next_sampler()
                .expect("glTF sampler exhaustion")
                .is_none()
        );

        for (source_index, (image, sampler)) in [(0, 0), (0, 1), (1, 0)].into_iter().enumerate() {
            let texture = appearance
                .next_texture()
                .expect("glTF texture read")
                .expect("declared glTF texture");
            assert_eq!(texture.source_index(), source_index);
            assert_eq!((texture.image(), texture.sampler()), (image, sampler));
        }
        assert!(
            appearance
                .next_texture()
                .expect("glTF texture exhaustion")
                .is_none()
        );

        for (source_index, (offset, count)) in [(0, 2), (2, 1)].into_iter().enumerate() {
            let material = appearance
                .next_material()
                .expect("glTF material read")
                .expect("declared glTF material");
            assert_eq!(material.source_index(), source_index);
            assert_eq!(
                (material.binding_offset(), material.binding_count()),
                (offset, count),
            );
        }
        assert!(
            appearance
                .next_material()
                .expect("glTF material exhaustion")
                .is_none()
        );

        let expected_bindings = [
            (0, TextureRoleKind::BaseColor),
            (2, TextureRoleKind::MetallicRoughness),
            (1, TextureRoleKind::Normal),
        ];
        for (source_index, (texture, role)) in expected_bindings.into_iter().enumerate() {
            let binding = appearance
                .next_binding()
                .expect("glTF material binding read")
                .expect("declared glTF material binding");
            assert_eq!(binding.source_index(), source_index);
            assert_eq!((binding.texture(), binding.role()), (texture, role));
        }
        assert!(
            appearance
                .next_binding()
                .expect("glTF material binding exhaustion")
                .is_none()
        );
    }

    {
        let geometry = cursor.geometry().expect("glTF geometry cursor");
        let logical_mesh = geometry
            .next_logical_mesh()
            .expect("glTF logical mesh read")
            .expect("declared glTF logical mesh");
        assert_eq!(logical_mesh.source_index(), 0);
        assert_eq!(
            (
                logical_mesh.primitive_offset(),
                logical_mesh.primitive_count()
            ),
            (0, 2)
        );
        assert!(
            geometry
                .next_logical_mesh()
                .expect("glTF logical mesh exhaustion")
                .is_none()
        );

        for source_index in 0..2 {
            let mut primitive = geometry
                .next_primitive()
                .expect("glTF primitive read")
                .expect("declared glTF primitive");
            assert_eq!(primitive.source_index(), source_index);
            assert_eq!(primitive.topology(), TopologyKind::TriangleList);
            assert_eq!(
                (
                    primitive.vertex_count(),
                    primitive.corner_count(),
                    primitive.face_count(),
                ),
                (3, 3, 1),
            );
            assert!(!primitive.polygon_boundaries_present());
            assert!(primitive.take_polygon_boundaries().is_none());
            assert!(primitive.indices_present());
            assert_eq!(primitive.index_count(), TRIANGLE_INDICES.len());
            assert_geometry_u32(
                primitive.take_indices().expect("glTF primitive indices"),
                &TRIANGLE_INDICES,
            );
            assert_eq!(primitive.material(), Some(source_index));
            assert_eq!(primitive.attribute_offset(), source_index * 3);
            assert_eq!(primitive.attribute_count(), 3);
        }
        assert!(
            geometry
                .next_primitive()
                .expect("glTF primitive exhaustion")
                .is_none()
        );

        let expected_semantics = [
            NodeSceneGeometryAttributeSemantic::Position,
            NodeSceneGeometryAttributeSemantic::Normal,
            NodeSceneGeometryAttributeSemantic::Texcoord(0),
            NodeSceneGeometryAttributeSemantic::Position,
            NodeSceneGeometryAttributeSemantic::Normal,
            NodeSceneGeometryAttributeSemantic::Texcoord(0),
        ];
        let expected_values: [&[f32]; 6] = [
            &POSITIONS, &NORMALS, &TEXCOORDS, &POSITIONS, &NORMALS, &TEXCOORDS,
        ];
        for source_index in 0..expected_semantics.len() {
            let attribute = geometry
                .next_attribute()
                .expect("glTF attribute read")
                .expect("declared glTF attribute");
            assert_eq!(attribute.source_index(), source_index);
            assert_eq!(attribute.semantic(), expected_semantics[source_index]);
            assert_eq!(attribute.domain(), AttributeDomain::Vertex);
            match attribute.into_payload() {
                NodeSceneGeometryAttributePayload::DenseF32 { values } => {
                    assert_geometry_f32(values, expected_values[source_index]);
                }
                _ => panic!("glTF capability attributes must retain dense f32 payloads"),
            }
        }
        assert!(
            geometry
                .next_attribute()
                .expect("glTF attribute exhaustion")
                .is_none()
        );

        let instance = geometry
            .next_mesh_instance()
            .expect("glTF mesh instance read")
            .expect("declared glTF mesh instance");
        assert_eq!(instance.source_index(), 0);
        assert_eq!((instance.owner_node(), instance.logical_mesh()), (0, 0));
        assert!(
            geometry
                .next_mesh_instance()
                .expect("glTF mesh instance exhaustion")
                .is_none()
        );
    }

    let mut skin_joint_count = 0;
    let mut influence_row_count = 0;
    let mut influence_count = 0;
    {
        let deformation = cursor.deformation().expect("glTF deformation cursor");
        let mut skin = deformation
            .next_skin()
            .expect("glTF skin read")
            .expect("declared glTF skin");
        assert_eq!(skin.source_index(), 0);
        assert_eq!(skin.name(), Some("Armature"));
        assert_eq!(skin.skeleton_root(), Some(1));
        assert_eq!(skin.matrix_spaces(), SkinMatrixSpaces::canonical());
        let mut joints = skin.take_joints().expect("glTF skin joints");
        for expected in [1, 2] {
            assert_eq!(joints.next().expect("glTF skin joint"), Some(expected));
            skin_joint_count += 1;
        }
        assert_eq!(joints.next().expect("glTF skin joint exhaustion"), None);
        joints.finish().expect("complete glTF skin joint traversal");
        let expected_inverse_bind = [
            Mat4::IDENTITY,
            Mat4 {
                cols: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, -1.0, 0.0, 1.0],
                ],
            },
        ];
        let mut matrices = skin
            .take_inverse_bind_matrices()
            .expect("glTF inverse-bind matrices");
        for expected in expected_inverse_bind {
            assert_eq!(
                matrices.next().expect("glTF inverse-bind matrix"),
                Some(expected)
            );
        }
        assert_eq!(matrices.next().expect("glTF inverse-bind exhaustion"), None,);
        matrices
            .finish()
            .expect("complete glTF inverse-bind traversal");
        assert!(
            deformation
                .next_skin()
                .expect("glTF skin exhaustion")
                .is_none()
        );

        let mut logical_morph = deformation
            .next_logical_morph()
            .expect("glTF logical morph read")
            .expect("declared glTF logical morph");
        assert_eq!(logical_morph.source_index(), 0);
        assert_eq!(
            (
                logical_morph.definition_offset(),
                logical_morph.definition_count(),
            ),
            (0, 2),
        );
        assert_deformation_f32(
            logical_morph
                .take_default_weights()
                .expect("glTF default morph weights"),
            &MORPH_DEFAULTS,
        );
        assert!(
            deformation
                .next_logical_morph()
                .expect("glTF logical morph exhaustion")
                .is_none()
        );

        for (source_index, expected_name) in ["Smile", "Blink"].into_iter().enumerate() {
            let definition = deformation
                .next_morph_definition()
                .expect("glTF morph definition read")
                .expect("declared glTF morph definition");
            assert_eq!(definition.source_index(), source_index);
            assert_eq!(definition.logical_mesh(), 0);
            assert_eq!(definition.target_ordinal(), source_index);
            assert_eq!(
                definition.stable_identity(),
                u32::try_from(source_index).expect("glTF morph-definition index must fit u32"),
            );
            assert_eq!(definition.name(), Some(expected_name));
        }
        assert!(
            deformation
                .next_morph_definition()
                .expect("glTF morph definition exhaustion")
                .is_none()
        );

        for (source_index, expected_influences) in [8, 3].into_iter().enumerate() {
            let mut primitive = deformation
                .next_primitive()
                .expect("glTF primitive deformation read")
                .expect("declared glTF primitive deformation");
            assert_eq!(primitive.source_index(), source_index);
            assert!(primitive.influences_present());
            assert_eq!(primitive.morph_target_offset(), source_index * 2);
            assert_eq!(primitive.morph_target_count(), 2);
            let mut rows = primitive.take_influences().expect("glTF influence rows");
            assert_eq!(rows.vertex_count(), 3);
            let before = influence_count;
            while let Some(row) = rows.next_row().expect("glTF influence row") {
                assert_eq!(row.vertex(), influence_row_count % 3);
                influence_row_count += 1;
                let mut influences = row.into_influences();
                while let Some(influence) = influences.next().expect("glTF influence") {
                    let _ = (
                        influence.palette_joint(),
                        influence.weight(),
                        influence.source_order(),
                    );
                    influence_count += 1;
                }
                influences.finish().expect("complete glTF influence row");
            }
            rows.finish()
                .expect("complete glTF influence-row traversal");
            assert_eq!(influence_count - before, expected_influences);
        }
        assert!(
            deformation
                .next_primitive()
                .expect("glTF primitive deformation exhaustion")
                .is_none()
        );

        for source_index in 0..4 {
            let target = deformation
                .next_morph_target()
                .expect("glTF morph target read")
                .expect("declared glTF morph target");
            assert_eq!(target.source_index(), source_index);
            assert_eq!(target.primitive(), source_index / 2);
            assert_eq!(target.target_ordinal(), source_index % 2);
            assert_eq!(target.delta_offset(), source_index);
            assert_eq!(target.delta_count(), 1);
        }
        assert!(
            deformation
                .next_morph_target()
                .expect("glTF morph target exhaustion")
                .is_none()
        );

        for source_index in 0..4 {
            let delta = deformation
                .next_morph_delta()
                .expect("glTF morph delta read")
                .expect("declared glTF morph delta");
            assert_eq!(delta.source_index(), source_index);
            assert_eq!(delta.morph_target(), source_index);
            assert_eq!(delta.semantic(), MorphDeltaSemantic::Position);
            match delta.into_payload() {
                NodeSceneDeformationMorphPayload::Dense {
                    logical_count,
                    mut values,
                } => {
                    assert!(source_index.is_multiple_of(2));
                    assert_eq!(logical_count, 3);
                    assert_eq!(values.declared_count(), 3);
                    while values.next().expect("glTF dense morph value").is_some() {}
                    values
                        .finish()
                        .expect("complete glTF dense morph traversal");
                }
                NodeSceneDeformationMorphPayload::Sparse {
                    logical_count,
                    mut values,
                } => {
                    assert!(!source_index.is_multiple_of(2));
                    assert_eq!(logical_count, 3);
                    assert_eq!(values.declared_count(), 1);
                    while let Some(value) = values.next().expect("glTF sparse morph value") {
                        let _ = (value.index(), value.value());
                    }
                    values
                        .finish()
                        .expect("complete glTF sparse morph traversal");
                }
            }
        }
        assert!(
            deformation
                .next_morph_delta()
                .expect("glTF morph delta exhaustion")
                .is_none()
        );

        let mut instance = deformation
            .next_mesh_instance()
            .expect("glTF deformation mesh instance read")
            .expect("declared glTF deformation mesh instance");
        assert_eq!(instance.source_index(), 0);
        assert_eq!(instance.skin(), Some(0));
        assert!(instance.morph_weights_present());
        assert_deformation_f32(
            instance
                .take_morph_weights()
                .expect("glTF instance morph weights"),
            &MORPH_OVERRIDE,
        );
        assert!(
            deformation
                .next_mesh_instance()
                .expect("glTF deformation mesh instance exhaustion")
                .is_none()
        );
    }

    let mut animation_key_count = 0;
    {
        let animation = cursor.animation().expect("glTF animation cursor");
        let clip = animation
            .next_clip()
            .expect("glTF animation clip read")
            .expect("declared glTF animation clip");
        assert_eq!(clip.source_index(), 0);
        assert_eq!(clip.duration(), 1.0);
        assert_eq!(clip.time_domain(), NodeSceneAnimationTimeDomain::Seconds);
        assert_eq!((clip.channel_offset(), clip.channel_count()), (0, 3));
        assert!(
            animation
                .next_clip()
                .expect("glTF animation clip exhaustion")
                .is_none()
        );

        let expected_targets = [
            NodeSceneAnimationTarget::NodeTranslation(1),
            NodeSceneAnimationTarget::NodeRotation(3),
            NodeSceneAnimationTarget::MeshInstanceMorphWeights(0),
        ];
        let expected_interpolations = [
            NodeSceneAnimationInterpolation::Linear,
            NodeSceneAnimationInterpolation::Step,
            NodeSceneAnimationInterpolation::CubicSpline,
        ];
        let expected_values: [&[f32]; 3] = [
            &TRANSLATION_VALUES,
            &ROTATION_VALUES,
            &MORPH_ANIMATION_VALUES,
        ];
        for source_index in 0..3 {
            let channel = animation
                .next_channel()
                .expect("glTF animation channel read")
                .expect("declared glTF animation channel");
            assert_eq!(channel.source_index(), source_index);
            assert_eq!(channel.target(), expected_targets[source_index]);
            assert_eq!(
                channel.interpolation(),
                expected_interpolations[source_index],
            );
            assert_eq!(
                channel.pre_extrapolation(),
                NodeSceneAnimationExtrapolation::SourceDefault,
            );
            assert_eq!(
                channel.post_extrapolation(),
                NodeSceneAnimationExtrapolation::SourceDefault,
            );
            assert_eq!(channel.times(), &KEY_TIMES);
            assert_eq!(channel.values(), expected_values[source_index]);
            animation_key_count += channel.key_count();
            if source_index == 2 {
                assert_eq!(channel.in_tangents(), Some(&MORPH_TANGENTS[..]));
                assert_eq!(channel.out_tangents(), Some(&MORPH_TANGENTS[..]));
            } else {
                assert_eq!(channel.in_tangents(), None);
                assert_eq!(channel.out_tangents(), None);
            }
        }
        assert!(
            animation
                .next_channel()
                .expect("glTF animation channel exhaustion")
                .is_none()
        );
    }
    cursor.finish().expect("complete glTF semantic traversal");

    assert_eq!(skin_joint_count, 2);
    assert_eq!(influence_row_count, 6);
    assert_eq!(influence_count, 11);
    assert_eq!(animation_key_count, 6);
    GltfCapabilityCanonicalShapeObservation {
        node_count,
        root_count,
        edge_count,
        image_count,
        sampler_count,
        texture_count,
        material_count,
        material_binding_count,
        logical_mesh_count,
        primitive_count,
        attribute_count,
        mesh_instance_count,
        skin_count,
        skin_joint_count,
        logical_morph_count,
        morph_definition_count,
        primitive_deformation_count,
        morph_target_count,
        morph_delta_count,
        influence_row_count,
        influence_count,
        animation_clip_count,
        animation_channel_count,
        animation_key_count,
    }
}

pub(in crate::canonical_scene::store) fn capture_gltf_capability_snapshots_for_probe(
    parts: &NodeSceneSuccessParts,
) -> (NodeSceneSnapshot, NodeSceneSnapshot) {
    let full = parts
        .scene
        .capture_snapshot(snapshot_limits(64 * 1024, 128), None)
        .expect("complete glTF snapshot with review preview");
    let bounded = parts
        .scene
        .capture_snapshot(snapshot_limits(96, 1), None)
        .expect("complete glTF snapshot with bounded preview");
    (full, bounded)
}

fn snapshot_limits(bytes: usize, items: usize) -> NodeSceneSnapshotLimits {
    NodeSceneSnapshotLimits::try_new(bytes, items, 1_000_000)
        .expect("glTF capability snapshot limits")
}

fn snapshot_mutation(
    account: &PhysicalStorageAccount,
    mutation: GltfCapabilityMutation,
) -> NodeSceneSnapshot {
    let parts = build_gltf_scene(account, mutation);
    let snapshot = parts
        .scene
        .capture_snapshot(snapshot_limits(0, 0), None)
        .expect("complete glTF capability digest");
    drop(parts);
    snapshot
}

fn assert_scope_publishes(scope: GltfCapabilityScope) {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let parts = build_gltf_scene_with_generation_source(
        &account,
        scope,
        GltfCapabilityMutation::Baseline,
        SceneGenerationId::reserve,
    );
    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

macro_rules! scope_publication_test {
    ($name:ident, $scope:ident) => {
        #[test]
        fn $name() {
            assert_scope_publishes(GltfCapabilityScope::$scope);
        }
    };
}

scope_publication_test!(gltf_image_seam_publishes, Images);
scope_publication_test!(gltf_image_payload_seam_publishes, ImagePayload);
scope_publication_test!(gltf_sampler_seam_publishes, Samplers);
scope_publication_test!(gltf_texture_seam_publishes, Textures);
scope_publication_test!(gltf_appearance_seam_publishes, Appearance);
scope_publication_test!(gltf_geometry_seam_publishes, Geometry);
scope_publication_test!(gltf_deformation_seam_publishes, Deformation);
scope_publication_test!(gltf_all_domain_seam_publishes, All);

#[test]
fn gltf_mixed_image_media_preserves_cumulative_offsets() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let parts = build_gltf_scene_with_generation_source(
        &account,
        GltfCapabilityScope::MixedImageMedia,
        GltfCapabilityMutation::Baseline,
        SceneGenerationId::reserve,
    );
    let mut cursor = parts
        .scene
        .semantic_view()
        .expect("mixed-media semantic view")
        .into_cursor()
        .expect("mixed-media semantic cursor");
    let mut roots = cursor.take_roots().expect("mixed-media roots");
    while roots.next().expect("mixed-media root").is_some() {}
    roots.finish().expect("complete mixed-media roots");
    while let Some(node) = cursor.next_node().expect("mixed-media node") {
        let mut children = node.into_children();
        while children.next().expect("mixed-media child").is_some() {}
        children.finish().expect("complete mixed-media children");
    }
    let appearance = cursor.appearance().expect("mixed-media appearance");
    let mut media_types = Vec::new();
    while let Some(image) = appearance.next_image().expect("mixed-media image") {
        media_types.push(image.media_type());
    }
    assert_eq!(media_types, [Some("image/png"), None, Some("image/png")]);
    cursor
        .finish()
        .expect("complete mixed-media semantic traversal");
    parts
        .scene
        .capture_snapshot(snapshot_limits(0, 0), None)
        .expect("mixed-media complete digest");
    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

fn reject_gltf_mutation(
    account: &PhysicalStorageAccount,
    mutation: GltfCapabilityMutation,
    inspect: impl FnOnce(&NodeSceneConstructionRejection),
) {
    let rejected = with_gltf_scene_input(GltfCapabilityScope::All, mutation, |input| {
        match SceneConstructionOperation::with_account_for_scene_input(
            account,
            test_limits(),
            input,
            SceneValidationLimits::default(),
        )
        .expect("invalid glTF shape operation admission")
        .try_validate()
        {
            Ok(_) => panic!("invalid glTF shape must not validate"),
            Err(rejected) => rejected,
        }
    });
    inspect(rejected.failure());
    let terminal = rejected
        .prepare_minimal_fallback()
        .expect("invalid glTF shape fallback")
        .handoff();
    assert!(matches!(
        &terminal,
        NodeSceneConstructionTerminal::MinimalFallback(_)
    ));
    drop(terminal);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn gltf_semantic_failures_are_attributed_to_the_owning_domain() {
    let factory = test_factory();
    let account = test_account(&factory, 512);

    reject_gltf_mutation(
        &account,
        GltfCapabilityMutation::EmptyImagePayload,
        |failure| match failure {
            NodeSceneConstructionRejection::AppearanceAuthoring(
                AppearanceAuthoringFailure::Semantic(
                    AppearanceAuthoringSemanticError::EmptyEncodedPayload { image: 0 },
                ),
            ) => {}
            other => panic!("unexpected appearance rejection: {other:?}"),
        },
    );
    reject_gltf_mutation(
        &account,
        GltfCapabilityMutation::SkinInverseBindCountMismatch,
        |failure| match failure {
            NodeSceneConstructionRejection::DeformationAuthoring(
                DeformationAuthoringFailure::Semantic(
                    DeformationAuthoringSemanticError::InverseBindCountMismatch {
                        skin: 0,
                        joint_count: 2,
                        inverse_bind_count: 1,
                    },
                ),
            ) => {}
            other => panic!("unexpected deformation rejection: {other:?}"),
        },
    );
    reject_gltf_mutation(
        &account,
        GltfCapabilityMutation::AnimationTimeOrder,
        |failure| match failure {
            NodeSceneConstructionRejection::AnimationAuthoring(
                AnimationAuthoringFailure::Semantic(
                    AnimationAuthoringSemanticError::TimesNotStrictlyIncreasing {
                        clip: 0,
                        channel: 0,
                        previous_key: 0,
                        key: 1,
                    },
                ),
            ) => {}
            other => panic!("unexpected animation rejection: {other:?}"),
        },
    );

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn gltf_construction_one_short_work_limit_publishes_only_rich_failure() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let baseline = build_gltf_scene(&account, GltfCapabilityMutation::Baseline);
    let successful_work = baseline
        .report
        .ledger_evidence()
        .dimension_by_semantic(STORE_DIMENSION)
        .expect("successful glTF work evidence")
        .attempted();
    assert_eq!(
        baseline
            .report
            .ledger_evidence()
            .dimension_by_semantic(STORE_DIMENSION)
            .expect("successful glTF work evidence")
            .consumed(),
        successful_work,
    );
    drop(baseline);

    let work_limit = successful_work
        .checked_sub(1)
        .expect("glTF construction must consume work");
    with_gltf_scene_input(
        GltfCapabilityScope::All,
        GltfCapabilityMutation::Baseline,
        |input| {
            let validated = SceneConstructionOperation::with_account_for_scene_input(
                &account,
                test_limits_with_work(work_limit),
                input,
                SceneValidationLimits::default(),
            )
            .expect("work-bounded glTF Scene input admission")
            .try_validate()
            .unwrap_or_else(|_| panic!("one-short total work must still admit glTF validation"));

            let failure = rich_failure(
                validated
                    .prepare()
                    .expect("one-short glTF work limit must prepare rich failure"),
            );
            let work = failure
                .ledger_evidence()
                .dimension_by_semantic(STORE_DIMENSION)
                .expect("failed glTF work evidence");
            assert_eq!(work.attempted(), successful_work);
            assert_eq!(work.consumed(), work_limit);
            assert_eq!(
                terminal_failure_class(failure.ledger_evidence()),
                StorePreparationFailureClass::Ledger,
            );
            assert_released(failure, &account);
        },
    );
}

#[test]
fn gltf_cancellation_after_validation_publishes_only_rich_failure() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let cancellation = CancellationToken::new();

    with_gltf_scene_input(
        GltfCapabilityScope::All,
        GltfCapabilityMutation::Baseline,
        |input| {
            let validated =
                SceneConstructionOperation::with_account_and_cancellation_for_scene_input_for_test(
                    &account,
                    test_limits(),
                    input,
                    SceneValidationLimits::default(),
                    cancellation.clone(),
                )
                .expect("cancellable glTF Scene input admission")
                .try_validate()
                .unwrap_or_else(|_| panic!("glTF input must validate before cancellation"));

            cancellation.cancel();
            let failure = rich_failure(
                validated
                    .prepare()
                    .expect("cancelled glTF construction must prepare rich failure"),
            );
            assert_eq!(
                terminal_failure_class(failure.ledger_evidence()),
                StorePreparationFailureClass::Control,
            );
            assert_released(failure, &account);
        },
    );
}

#[test]
fn gltf_shape_materializes_all_five_canonical_domains() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let parts = build_gltf_scene(&account, GltfCapabilityMutation::Baseline);
    let shape = inspect_gltf_capability_semantics_for_probe(&parts);
    assert_eq!((shape.node_count, shape.image_count), (3, 2));
    assert_eq!((shape.primitive_count, shape.skin_count), (2, 1));
    assert_eq!(
        (shape.morph_target_count, shape.animation_channel_count),
        (4, 3)
    );

    let (full, bounded) = capture_gltf_capability_snapshots_for_probe(&parts);
    assert_eq!(full.schema_revision, 3);
    assert_eq!(bounded.digest, full.digest);
    assert_eq!(bounded.totals, full.totals);
    assert!(bounded.preview.bytes_truncated || bounded.preview.items_truncated);
    assert_eq!(full.totals.skin_joint_count, 2);
    assert_eq!(full.totals.inverse_bind_matrix_count, 2);
    assert_eq!(full.totals.influence_row_count, 6);
    assert_eq!(full.totals.influence_count, 11);
    assert_eq!(full.totals.morph_value_vector_count, 8);
    assert_eq!(full.totals.morph_sparse_index_count, 2);
    assert_eq!(full.totals.animation_time_scalar_count, 6);
    assert_eq!(full.totals.animation_value_scalar_count, 18);
    assert_eq!(full.totals.animation_in_tangent_scalar_count, 4);
    assert_eq!(full.totals.animation_out_tangent_scalar_count, 4);
    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn gltf_shape_digest_is_generation_independent_and_field_complete() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let baseline = snapshot_mutation(&account, GltfCapabilityMutation::Baseline);
    let equivalent = snapshot_mutation(&account, GltfCapabilityMutation::Baseline);
    assert_eq!(baseline.digest, equivalent.digest);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);

    let observations = collect_gltf_capability_vectors_for_probe();
    assert_eq!(observations.baseline_snapshot_digest, baseline.digest);
    let mut mutation_digests = Vec::new();
    let mut mutation_vectors = Vec::new();
    for mutation in observations.mutations {
        assert_ne!(mutation.snapshot_digest, baseline.digest);
        assert_ne!(mutation.source_vector, observations.source_vector);
        assert!(!mutation_digests.contains(&mutation.snapshot_digest));
        assert!(!mutation_vectors.contains(&mutation.source_vector));
        mutation_digests.push(mutation.snapshot_digest);
        mutation_vectors.push(mutation.source_vector);
    }
}
