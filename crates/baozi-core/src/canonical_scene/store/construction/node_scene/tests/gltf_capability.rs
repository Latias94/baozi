use std::sync::atomic::AtomicU64;

use sha2::{Digest, Sha256};

use baozi_operation::{
    CancellationToken, PhysicalStorageAccount, StorePreparationFailureClass, StoreRuntimeError,
};

use crate::SceneValidationLimits;
use crate::appearance::{
    AlphaMode, ColorInterpretation, MaterialFactors, MaterialSemanticsRef,
    PbrMetallicRoughnessFactors, PbrSpecularGlossinessFactors, ShadingModel,
    SupplementalPbrFactors, TextureBindingSemanticsRef, TextureBlendOperation, TextureChannel,
    TextureChannelMapping, TextureChannelSwizzle, TextureCoordinateSource, TextureRole,
    TextureSourceKey, TextureStrength, TextureTransform,
};
use crate::attribute::{AttributeDomain, AttributeSemantic};
use crate::canonical_scene::store::construction::node_scene::semantic::{
    NodeSceneAnimationExtrapolation, NodeSceneAnimationInterpolation, NodeSceneAnimationTarget,
    NodeSceneAnimationTimeDomain, NodeSceneDeformationF32Cursor, NodeSceneDeformationMorphPayload,
    NodeSceneGeometryAttributePayload, NodeSceneGeometryAttributeSemantic,
    NodeSceneGeometryF32Cursor, NodeSceneGeometryI32Cursor, NodeSceneGeometryU32Cursor,
    NodeSceneProfileStep, NodeSceneSemanticResult,
};
use crate::canonical_scene::store::construction::node_scene::snapshot::{
    NodeSceneSnapshot, NodeSceneSnapshotLimits,
};
use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneConstructionTerminal, NodeSceneDraftBuild, NodeSceneSuccessParts,
    PreparedNodeSceneConstruction, ValidatedNodeScene,
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
    SkinAuthoring, SkinLocalId, SpatialAuthoringBatch, TextureAuthoring, TextureLocalId,
};
use crate::deform::{MorphDeltaSemantic, SkinMatrixSpace, SkinMatrixSpaces};
use crate::entity::{AuthoredTransform, AuthoredTransformKind, EntityName};
use crate::extension::ExtensionMediaType;
use crate::geometry::TopologyKind;
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};
use crate::image::{MipmapFilter, SamplerState, TexelFilter, TextureAddressMode};
use crate::math::{Color, Mat4, Quat, Vec2, Vec3};
use crate::scene_space::SceneSpace;

use super::{
    NodeSceneConstructionRejection, assert_released, exhaust_profile_for_test, profile_fixture,
    rich_failure, terminal_failure_class, terminal_failure_code, test_account, test_factory,
    test_limits, test_limits_with_work,
};

pub(in crate::canonical_scene::store) const GLTF_SOURCE_VECTOR_SCHEMA_ID: &str =
    "baozi.selected-gltf-shape-capability-source.v2";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GltfCapabilityMutation {
    Baseline,
    NodeTranslation,
    ImagePayload,
    EmptyImagePayload,
    MaterialBindingSemantics,
    MaterialSemantics,
    PrimitiveMaterial,
    SkinInverseBind,
    SkinInverseBindCountMismatch,
    PaletteJointOutOfRange,
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
    pub(in crate::canonical_scene::store) mutations: [GltfCapabilityMutationObservation; 9],
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

pub(in crate::canonical_scene::store) const FACADE_OBSERVATION_SCHEMA_ID: &str =
    "baozi.facade-observation.v1";

/// A fixed-size facade projection over the complete typed semantic traversal.
///
/// The digest and byte count commit the stable canonical semantic transcript. The shape and proof
/// booleans are produced only after the facade-intended typed cursor has observed and exhausted all
/// roots, nodes, appearance, geometry, deformation, animation, and profile values and references.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct FacadeObservationV1 {
    pub(in crate::canonical_scene::store) schema_id: &'static str,
    pub(in crate::canonical_scene::store) transcript_digest: [u8; 32],
    pub(in crate::canonical_scene::store) transcript_bytes: u64,
    pub(in crate::canonical_scene::store) totals: GltfCapabilityCanonicalShapeObservation,
    pub(in crate::canonical_scene::store) reference_closure_verified: bool,
    pub(in crate::canonical_scene::store) cursor_exhausted: bool,
}

/// A bounded, inert projection of one typed facade-read failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct FacadeObservationErrorV1 {
    pub(in crate::canonical_scene::store) schema_id: &'static str,
    pub(in crate::canonical_scene::store) error_class: &'static str,
    pub(in crate::canonical_scene::store) reason: &'static str,
}

pub(in crate::canonical_scene::store) type FacadeObservationOutcomeV1 =
    Result<FacadeObservationV1, FacadeObservationErrorV1>;

struct FacadeTranscript {
    digest: Sha256,
    bytes: u64,
}

impl FacadeTranscript {
    fn new() -> Self {
        let mut transcript = Self {
            digest: Sha256::new(),
            bytes: 0,
        };
        transcript.debug("schema", &FACADE_OBSERVATION_SCHEMA_ID);
        transcript
    }

    fn debug<T: std::fmt::Debug>(&mut self, tag: &'static str, value: &T) {
        use std::fmt::Write as _;

        writeln!(self, "{tag}:{value:?}").expect("facade transcript hashing is infallible");
    }

    fn finish(self) -> ([u8; 32], u64) {
        (self.digest.finalize().into(), self.bytes)
    }
}

impl std::fmt::Write for FacadeTranscript {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        self.digest.update(value.as_bytes());
        self.bytes = self
            .bytes
            .checked_add(u64::try_from(value.len()).map_err(|_| std::fmt::Error)?)
            .ok_or(std::fmt::Error)?;
        Ok(())
    }
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

    fn i32(&mut self, value: i32) {
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

    fn encode_i32_slice(encoder: &mut GltfSourceEncoder, values: &[i32]) {
        encoder.list(values, |encoder, value| encoder.i32(*value));
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
            TextureRole::Displacement => 10,
            TextureRole::Clearcoat => 11,
            TextureRole::ClearcoatRoughness => 12,
            TextureRole::ClearcoatNormal => 13,
            TextureRole::Transmission => 14,
            TextureRole::Thickness => 15,
            TextureRole::SheenColor => 16,
            TextureRole::SheenRoughness => 17,
            TextureRole::Namespaced { .. } => 18,
        };
        encoder.u8(tag);
        if let TextureRole::Namespaced { schema_id } = role {
            encoder.utf8(schema_id.name().as_str());
            encoder.u32(schema_id.version().get());
        }
    }

    fn encode_schema_id(
        encoder: &mut GltfSourceEncoder,
        schema_id: &crate::extension::ExtensionSchemaId,
    ) {
        encoder.utf8(schema_id.name().as_str());
        encoder.u32(schema_id.version().get());
    }

    fn encode_option<T>(
        encoder: &mut GltfSourceEncoder,
        value: Option<T>,
        encode: impl FnOnce(&mut GltfSourceEncoder, T),
    ) {
        match value {
            Some(value) => {
                encoder.u8(1);
                encode(encoder, value);
            }
            None => encoder.u8(0),
        }
    }

    fn encode_vec3(encoder: &mut GltfSourceEncoder, value: Vec3) {
        encoder.f32(value.x);
        encoder.f32(value.y);
        encoder.f32(value.z);
    }

    fn encode_color(encoder: &mut GltfSourceEncoder, value: Color) {
        encoder.f32(value.r);
        encoder.f32(value.g);
        encoder.f32(value.b);
        encoder.f32(value.a);
    }

    fn encode_shading_model(encoder: &mut GltfSourceEncoder, model: &ShadingModel) {
        let tag = match model {
            ShadingModel::Unspecified => 7,
            ShadingModel::Unlit => 0,
            ShadingModel::Lambert => 1,
            ShadingModel::Phong => 2,
            ShadingModel::BlinnPhong => 3,
            ShadingModel::PbrMetallicRoughness => 4,
            ShadingModel::PbrSpecularGlossiness => 5,
            ShadingModel::Namespaced { .. } => 6,
        };
        encoder.u8(tag);
        if let ShadingModel::Namespaced { schema_id } = model {
            encode_schema_id(encoder, schema_id);
        }
    }

    fn encode_alpha_mode(encoder: &mut GltfSourceEncoder, alpha_mode: &AlphaMode) {
        match alpha_mode {
            AlphaMode::Opaque => encoder.u8(0),
            AlphaMode::Mask(cutoff) => {
                encoder.u8(1);
                encoder.f32(cutoff.get());
            }
            AlphaMode::Blend => encoder.u8(2),
            AlphaMode::Namespaced { schema_id } => {
                encoder.u8(3);
                encode_schema_id(encoder, schema_id);
            }
        }
    }

    fn encode_coordinate_source(encoder: &mut GltfSourceEncoder, source: &TextureCoordinateSource) {
        match source {
            TextureCoordinateSource::UvSet(set) => {
                encoder.u8(0);
                encoder.u32(u32::from(*set));
            }
            TextureCoordinateSource::ObjectPosition => encoder.u8(1),
            TextureCoordinateSource::ObjectNormal => encoder.u8(2),
            TextureCoordinateSource::ReflectionVector => encoder.u8(3),
            TextureCoordinateSource::Namespaced { schema_id } => {
                encoder.u8(4);
                encode_schema_id(encoder, schema_id);
            }
        }
    }

    fn encode_channel(encoder: &mut GltfSourceEncoder, channel: TextureChannel) {
        encoder.u8(match channel {
            TextureChannel::Red => 0,
            TextureChannel::Green => 1,
            TextureChannel::Blue => 2,
            TextureChannel::Alpha => 3,
            TextureChannel::Zero => 4,
            TextureChannel::One => 5,
        });
    }

    fn encode_channel_mapping(encoder: &mut GltfSourceEncoder, mapping: &TextureChannelMapping) {
        match mapping {
            TextureChannelMapping::Swizzle(swizzle) => {
                encoder.u8(0);
                encoder.list(swizzle.channels(), |encoder, channel| {
                    encode_channel(encoder, *channel);
                });
            }
            TextureChannelMapping::Namespaced { schema_id } => {
                encoder.u8(1);
                encode_schema_id(encoder, schema_id);
            }
        }
    }

    fn encode_color_interpretation(
        encoder: &mut GltfSourceEncoder,
        interpretation: &ColorInterpretation,
    ) {
        let tag = match interpretation {
            ColorInterpretation::SrgbColor => 0,
            ColorInterpretation::LinearColor => 1,
            ColorInterpretation::Data => 2,
            ColorInterpretation::Namespaced { .. } => 3,
        };
        encoder.u8(tag);
        if let ColorInterpretation::Namespaced { schema_id } = interpretation {
            encode_schema_id(encoder, schema_id);
        }
    }

    fn encode_texture_transform(encoder: &mut GltfSourceEncoder, transform: TextureTransform) {
        let offset = transform.offset();
        encoder.f32(offset.x);
        encoder.f32(offset.y);
        encoder.f32(transform.rotation_radians());
        let scale = transform.scale();
        encoder.f32(scale.x);
        encoder.f32(scale.y);
    }

    fn encode_blend_operation(encoder: &mut GltfSourceEncoder, operation: &TextureBlendOperation) {
        let tag = match operation {
            TextureBlendOperation::Replace => 0,
            TextureBlendOperation::Multiply => 1,
            TextureBlendOperation::Add => 2,
            TextureBlendOperation::Subtract => 3,
            TextureBlendOperation::Screen => 4,
            TextureBlendOperation::Namespaced { .. } => 5,
        };
        encoder.u8(tag);
        if let TextureBlendOperation::Namespaced { schema_id } = operation {
            encode_schema_id(encoder, schema_id);
        }
    }

    fn encode_material_factors(encoder: &mut GltfSourceEncoder, factors: MaterialFactors) {
        encode_vec3(encoder, factors.emissive());
        encode_option(
            encoder,
            factors.pbr_metallic_roughness(),
            |encoder, group| {
                encode_color(encoder, group.base_color());
                encoder.f32(group.metallic());
                encoder.f32(group.roughness());
            },
        );
        encode_option(
            encoder,
            factors.pbr_specular_glossiness(),
            |encoder, group| {
                encode_color(encoder, group.diffuse());
                encode_vec3(encoder, group.specular());
                encoder.f32(group.glossiness());
            },
        );
        encode_option(encoder, factors.legacy(), |encoder, group| {
            encode_vec3(encoder, group.ambient());
            encode_vec3(encoder, group.diffuse());
            encode_vec3(encoder, group.specular());
            encode_vec3(encoder, group.emissive());
            encoder.f32(group.shininess());
            encoder.f32(group.opacity());
        });
        let supplemental = factors.supplemental_pbr();
        encoder.f32(supplemental.clearcoat());
        encoder.f32(supplemental.clearcoat_roughness());
        encoder.f32(supplemental.transmission());
        encoder.f32(supplemental.thickness());
        encode_vec3(encoder, supplemental.sheen_color());
        encoder.f32(supplemental.sheen_roughness());
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
        let semantics = binding.semantics();
        encode_texture_role(encoder, semantics.role());
        encode_coordinate_source(encoder, semantics.coordinate_source());
        encode_channel_mapping(encoder, semantics.channel_mapping());
        encode_color_interpretation(encoder, semantics.color_interpretation());
        encode_texture_transform(encoder, semantics.transform());
        encoder.f32(semantics.strength().get());
        encode_blend_operation(encoder, semantics.blend());
        encode_option(encoder, semantics.source_key(), |encoder, source_key| {
            encoder.utf8(source_key.as_str());
        });
    });
    encoder.list(appearance.materials(), |encoder, material| {
        let bindings = material.bindings();
        encoder.u32(bindings.offset());
        encoder.u32(bindings.count());
        let semantics = material.semantics();
        encode_shading_model(encoder, semantics.shading_model());
        encode_alpha_mode(encoder, semantics.alpha_mode());
        encoder.u8(u8::from(semantics.double_sided()));
        encode_material_factors(encoder, semantics.factors());
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
            GeometryAttributePayloadAuthoring::DenseI32 { lanes, values } => {
                encoder.u8(4);
                encoder.u8(lanes);
                encode_i32_slice(encoder, values);
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
            GeometryAttributePayloadAuthoring::SparseI32 {
                lanes,
                logical_count,
                indices,
                values,
                default,
            } => {
                encoder.u8(5);
                encoder.u8(lanes);
                encoder.u32(logical_count);
                encode_u32_slice(encoder, indices);
                encode_i32_slice(encoder, values);
                encode_i32_slice(encoder, default);
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
        NodeAuthoringNode::new(
            Presence::Absent,
            node_zero_transform,
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            Presence::Present(NodeParent::new(NodeLocalId::new(1), 0)),
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
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
    let mut coordinate_sources = [
        TextureCoordinateSource::uv_set(0).expect("glTF UV set zero must be canonical"),
        TextureCoordinateSource::uv_set(0).expect("glTF UV set zero must be canonical"),
        TextureCoordinateSource::uv_set(0).expect("glTF UV set zero must be canonical"),
    ];
    let mut channel_mappings = [
        TextureChannelMapping::swizzle(TextureChannelSwizzle::RGBA),
        TextureChannelMapping::swizzle(
            TextureChannelSwizzle::try_new(&[TextureChannel::Blue, TextureChannel::Green])
                .expect("glTF metallic-roughness mapping must be canonical"),
        ),
        TextureChannelMapping::swizzle(TextureChannelSwizzle::RGB),
    ];
    let mut color_interpretations = [
        ColorInterpretation::SrgbColor,
        ColorInterpretation::Data,
        ColorInterpretation::Data,
    ];
    let mut transforms = [TextureTransform::IDENTITY; 3];
    let mut strengths = [TextureStrength::ONE; 3];
    let mut source_keys = [None, None, None];
    if mutation == GltfCapabilityMutation::MaterialBindingSemantics {
        coordinate_sources[0] =
            TextureCoordinateSource::uv_set(1).expect("glTF UV set one must be canonical");
        channel_mappings[0] = TextureChannelMapping::swizzle(TextureChannelSwizzle::RGB);
        color_interpretations[0] = ColorInterpretation::LinearColor;
        transforms[0] =
            TextureTransform::try_new(Vec2::new(0.125, 0.25), 0.375, Vec2::new(2.0, 0.5))
                .expect("finite glTF texture transform mutation");
        strengths[0] = TextureStrength::try_new(0.625).expect("finite glTF texture strength");
        source_keys[0] = Some(
            TextureSourceKey::try_new("baseColorTexture")
                .expect("glTF texture source key must be canonical"),
        );
    }
    let blends = [
        TextureBlendOperation::Replace,
        TextureBlendOperation::Replace,
        TextureBlendOperation::Replace,
    ];
    let binding_semantics = [
        TextureBindingSemanticsRef::new(
            &roles[0],
            &coordinate_sources[0],
            &channel_mappings[0],
            &color_interpretations[0],
            transforms[0],
            strengths[0],
            &blends[0],
            source_keys[0].as_ref(),
        ),
        TextureBindingSemanticsRef::new(
            &roles[1],
            &coordinate_sources[1],
            &channel_mappings[1],
            &color_interpretations[1],
            transforms[1],
            strengths[1],
            &blends[1],
            source_keys[1].as_ref(),
        ),
        TextureBindingSemanticsRef::new(
            &roles[2],
            &coordinate_sources[2],
            &channel_mappings[2],
            &color_interpretations[2],
            transforms[2],
            strengths[2],
            &blends[2],
            source_keys[2].as_ref(),
        ),
    ];
    let bindings = [
        MaterialBindingAuthoring::new(TextureLocalId::new(0), binding_semantics[0]),
        MaterialBindingAuthoring::new(TextureLocalId::new(2), binding_semantics[1]),
        MaterialBindingAuthoring::new(TextureLocalId::new(1), binding_semantics[2]),
    ];
    let material_semantics_mutated = mutation == GltfCapabilityMutation::MaterialSemantics;
    let shading_model = if material_semantics_mutated {
        ShadingModel::PbrSpecularGlossiness
    } else {
        ShadingModel::PbrMetallicRoughness
    };
    let alpha_mode = if material_semantics_mutated {
        AlphaMode::mask(0.4).expect("glTF alpha cutoff must be canonical")
    } else {
        AlphaMode::Opaque
    };
    let factors = if material_semantics_mutated {
        MaterialFactors::try_new_with_supplemental_pbr(
            Vec3::new(0.1, 0.2, 0.3),
            Some(
                PbrMetallicRoughnessFactors::try_new(
                    Color::linear_rgba(0.8, 0.7, 0.6, 0.5),
                    0.25,
                    0.75,
                )
                .expect("mutated glTF metallic-roughness factors must be canonical"),
            ),
            Some(
                PbrSpecularGlossinessFactors::try_new(
                    Color::linear_rgba(0.9, 0.8, 0.7, 1.0),
                    Vec3::new(0.2, 0.3, 0.4),
                    0.55,
                )
                .expect("mutated glTF specular-glossiness factors must be canonical"),
            ),
            None,
            SupplementalPbrFactors::try_new(0.2, 0.3, 0.4, 0.5, Vec3::new(0.1, 0.2, 0.3), 0.6)
                .expect("mutated glTF supplemental PBR factors must be canonical"),
        )
        .expect("mutated glTF material factors must be canonical")
    } else {
        MaterialFactors::try_new(
            Vec3::ZERO,
            Some(PbrMetallicRoughnessFactors::default()),
            None,
            None,
        )
        .expect("default glTF PBR factors must be canonical")
    };
    let material_semantics = MaterialSemanticsRef::new(
        &shading_model,
        &alpha_mode,
        material_semantics_mutated,
        factors,
    );
    let materials = [
        MaterialAuthoring::new(AuthoringRange::new(0, 2), material_semantics),
        MaterialAuthoring::new(AuthoringRange::new(2, 1), material_semantics),
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
    if mutation == GltfCapabilityMutation::PaletteJointOutOfRange {
        primitive_zero_influences[0] = InfluenceAuthoring::new(2, 0.40, 0);
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
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let profile = profile_fixture(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
    );
    use_input(
        SceneAuthoringBatch::new(
            spatial,
            node_batch,
            appearance,
            geometry,
            deformation,
            animations,
            profile.batch(),
        ),
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
        build_gltf_scene_input_with_generation_source(
            account,
            input,
            scope,
            mutation,
            reserve_generation,
        )
    })
}

fn build_gltf_scene_input_with_generation_source(
    account: &PhysicalStorageAccount,
    input: SceneAuthoringBatch<'_>,
    scope: GltfCapabilityScope,
    mutation: GltfCapabilityMutation,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> NodeSceneSuccessParts {
    let prepared = SceneConstructionOperation::with_account_for_scene_input(
        account,
        test_limits(),
        input,
        SceneValidationLimits::default(),
    )
    .expect("selected normalized glTF shape admission")
    .finish_draft()
    .expect("selected normalized glTF shape activation")
    .prepare_with_generation_source_for_test(reserve_generation)
    .expect("selected normalized glTF shape publication");
    match prepared.handoff() {
        NodeSceneConstructionTerminal::Success(parts) => parts,
        NodeSceneConstructionTerminal::RichFailure(_) => {
            panic!("{scope:?} glTF shape published rich failure")
        }
        NodeSceneConstructionTerminal::MinimalFallback(fallback) => {
            panic!(
                "{scope:?} glTF {mutation:?} shape published minimal fallback: class={:?}, code={}",
                terminal_failure_class(fallback.ledger_evidence()),
                terminal_failure_code(fallback.ledger_evidence()),
            )
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
                mutation,
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
            "material_binding_semantics",
            GltfCapabilityMutation::MaterialBindingSemantics,
        ),
        (
            "material_semantics",
            GltfCapabilityMutation::MaterialSemantics,
        ),
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
    scene: &ValidatedNodeScene,
) -> GltfCapabilityCanonicalShapeObservation {
    inspect_gltf_capability_semantics_with_first_transform_for_probe(
        scene,
        AuthoredTransform::identity_trs(),
    )
}

pub(in crate::canonical_scene::store) fn inspect_gltf_capability_semantics_with_first_transform_for_probe(
    scene: &ValidatedNodeScene,
    first_transform: AuthoredTransform,
) -> GltfCapabilityCanonicalShapeObservation {
    let view = scene
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
    let expected_transforms = [
        first_transform,
        AuthoredTransform::identity_trs(),
        AuthoredTransform::identity_trs(),
    ];
    for (source_index, (expected_children, expected_transform)) in expected_children
        .into_iter()
        .zip(expected_transforms)
        .enumerate()
    {
        let node = cursor
            .next_node()
            .expect("glTF node read")
            .expect("declared glTF node");
        assert_eq!(node.source_index(), source_index);
        assert!(
            node.authored_transform()
                .has_same_authored_bits(expected_transform)
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
                material.shading_model(),
                &ShadingModel::PbrMetallicRoughness,
            );
            assert_eq!(material.alpha_mode(), &AlphaMode::Opaque);
            assert!(!material.double_sided());
            let factors = material.factors();
            assert_eq!(factors.emissive(), Vec3::ZERO);
            assert_eq!(
                factors.pbr_metallic_roughness(),
                Some(PbrMetallicRoughnessFactors::default()),
            );
            assert_eq!(factors.pbr_specular_glossiness(), None);
            assert_eq!(factors.legacy(), None);
            assert_eq!(
                factors.supplemental_pbr(),
                SupplementalPbrFactors::default(),
            );
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
            (
                0,
                TextureRole::BaseColor,
                TextureCoordinateSource::uv_set(0).expect("glTF UV set zero"),
                TextureChannelMapping::swizzle(TextureChannelSwizzle::RGBA),
                ColorInterpretation::SrgbColor,
            ),
            (
                2,
                TextureRole::MetallicRoughness,
                TextureCoordinateSource::uv_set(0).expect("glTF UV set zero"),
                TextureChannelMapping::swizzle(
                    TextureChannelSwizzle::try_new(&[TextureChannel::Blue, TextureChannel::Green])
                        .expect("glTF metallic-roughness mapping"),
                ),
                ColorInterpretation::Data,
            ),
            (
                1,
                TextureRole::Normal,
                TextureCoordinateSource::uv_set(0).expect("glTF UV set zero"),
                TextureChannelMapping::swizzle(TextureChannelSwizzle::RGB),
                ColorInterpretation::Data,
            ),
        ];
        for (source_index, (texture, role, coordinate_source, channel_mapping, interpretation)) in
            expected_bindings.into_iter().enumerate()
        {
            let binding = appearance
                .next_binding()
                .expect("glTF material binding read")
                .expect("declared glTF material binding");
            assert_eq!(binding.source_index(), source_index);
            assert_eq!(binding.texture(), texture);
            assert_eq!(binding.role_semantics(), &role);
            assert_eq!(binding.coordinate_source(), &coordinate_source);
            assert_eq!(binding.channel_mapping(), &channel_mapping);
            assert_eq!(binding.color_interpretation(), &interpretation);
            assert_eq!(binding.transform(), TextureTransform::IDENTITY);
            assert_eq!(binding.strength(), TextureStrength::ONE);
            assert_eq!(binding.blend(), &TextureBlendOperation::Replace);
            assert_eq!(binding.source_key(), None);
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
    exhaust_profile_for_test(cursor.profile().expect("glTF profile cursor"));
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

fn observe_facade_typed_view(
    scene: &ValidatedNodeScene,
) -> NodeSceneSemanticResult<(GltfCapabilityCanonicalShapeObservation, bool, [u8; 32], u64)> {
    fn mark_index(valid: &mut bool, value: usize, upper: usize) {
        *valid &= value < upper;
    }

    fn mark_optional_index(valid: &mut bool, value: Option<usize>, upper: usize) {
        if let Some(value) = value {
            mark_index(valid, value, upper);
        }
    }

    fn mark_range(valid: &mut bool, offset: usize, count: usize, upper: usize) {
        *valid &= offset.checked_add(count).is_some_and(|end| end <= upper);
    }

    fn drain_geometry_f32(
        mut values: NodeSceneGeometryF32Cursor<'_>,
        transcript: &mut FacadeTranscript,
        label: &'static str,
    ) -> NodeSceneSemanticResult<usize> {
        let mut count: usize = 0;
        while let Some(value) = values.next()? {
            transcript.debug(label, &value);
            count = count
                .checked_add(1)
                .expect("facade geometry F32 observation count must fit usize");
        }
        values.finish()?;
        Ok(count)
    }

    fn drain_geometry_u32(
        mut values: NodeSceneGeometryU32Cursor<'_>,
        transcript: &mut FacadeTranscript,
        label: &'static str,
    ) -> NodeSceneSemanticResult<usize> {
        let mut count: usize = 0;
        while let Some(value) = values.next()? {
            transcript.debug(label, &value);
            count = count
                .checked_add(1)
                .expect("facade geometry U32 observation count must fit usize");
        }
        values.finish()?;
        Ok(count)
    }

    fn drain_geometry_i32(
        mut values: NodeSceneGeometryI32Cursor<'_>,
        transcript: &mut FacadeTranscript,
        label: &'static str,
    ) -> NodeSceneSemanticResult<usize> {
        let mut count: usize = 0;
        while let Some(value) = values.next()? {
            transcript.debug(label, &value);
            count = count
                .checked_add(1)
                .expect("facade geometry I32 observation count must fit usize");
        }
        values.finish()?;
        Ok(count)
    }

    fn drain_deformation_f32(
        mut values: NodeSceneDeformationF32Cursor<'_>,
        transcript: &mut FacadeTranscript,
        label: &'static str,
    ) -> NodeSceneSemanticResult<usize> {
        let mut count: usize = 0;
        while let Some(value) = values.next()? {
            transcript.debug(label, &value);
            count = count
                .checked_add(1)
                .expect("facade deformation F32 observation count must fit usize");
        }
        values.finish()?;
        Ok(count)
    }

    fn drain_profile_steps<T: std::fmt::Debug>(
        mut step: impl FnMut() -> NodeSceneSemanticResult<NodeSceneProfileStep<T>>,
        transcript: &mut FacadeTranscript,
        label: &'static str,
    ) -> NodeSceneSemanticResult<()> {
        loop {
            match step()? {
                NodeSceneProfileStep::Checkpoint => {}
                NodeSceneProfileStep::Item(item) => transcript.debug(label, &item),
                NodeSceneProfileStep::End => return Ok(()),
            }
        }
    }

    let view = scene.semantic_view()?;
    let mut transcript = FacadeTranscript::new();
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
    let mut reference_closure = true;
    let mut structural_reference_count = 0usize;
    let mut skin_joint_count = 0usize;
    let mut influence_row_count = 0usize;
    let mut influence_count = 0usize;
    let mut animation_key_count = 0usize;

    transcript.debug("scene-space", &view.scene_space()?);
    transcript.debug("camera-count", &view.camera_count());
    transcript.debug("light-count", &view.light_count());
    for index in 0..view.camera_count() {
        transcript.debug("camera", &view.camera_at(index)?);
    }
    for index in 0..view.light_count() {
        transcript.debug("light", &view.light_at(index)?);
    }
    let mut placements = view.spatial_placements();
    while let Some(placement) = placements.next()? {
        transcript.debug("spatial-placement", &placement);
    }
    placements.finish()?;

    let mut cursor = view.into_cursor()?;
    let mut roots = cursor.take_roots()?;
    while let Some(root) = roots.next()? {
        transcript.debug("root", &root);
        mark_index(&mut reference_closure, root, node_count);
        structural_reference_count = structural_reference_count
            .checked_add(1)
            .expect("facade structural reference count must fit usize");
    }
    roots.finish()?;

    while let Some(node) = cursor.next_node()? {
        transcript.debug("node", &(node.source_index(), node.authored_transform()));
        let mut children = node.into_children();
        while let Some(child) = children.next()? {
            transcript.debug("node-child", &child);
            mark_index(&mut reference_closure, child, node_count);
            structural_reference_count = structural_reference_count
                .checked_add(1)
                .expect("facade structural reference count must fit usize");
        }
        children.finish()?;
    }

    {
        let appearance = cursor.appearance()?;
        while let Some(image) = appearance.next_image()? {
            transcript.debug(
                "image",
                &(
                    image.source_index(),
                    image.encoded_payload(),
                    image.media_type(),
                ),
            );
        }
        while let Some(sampler) = appearance.next_sampler()? {
            let state = sampler.state();
            transcript.debug(
                "sampler",
                &(
                    sampler.source_index(),
                    state.minification(),
                    state.magnification(),
                    state.mipmap(),
                    state.address_u(),
                    state.address_v(),
                    state.address_w(),
                ),
            );
        }
        while let Some(texture) = appearance.next_texture()? {
            transcript.debug(
                "texture",
                &(texture.source_index(), texture.image(), texture.sampler()),
            );
            mark_index(&mut reference_closure, texture.image(), image_count);
            mark_index(&mut reference_closure, texture.sampler(), sampler_count);
        }
        while let Some(material) = appearance.next_material()? {
            transcript.debug(
                "material",
                &(
                    material.source_index(),
                    material.shading_model(),
                    material.alpha_mode(),
                    material.double_sided(),
                    material.factors(),
                    material.factor_values(),
                    material.binding_offset(),
                    material.binding_count(),
                ),
            );
            mark_range(
                &mut reference_closure,
                material.binding_offset(),
                material.binding_count(),
                material_binding_count,
            );
        }
        while let Some(binding) = appearance.next_binding()? {
            transcript.debug(
                "binding",
                &(
                    binding.source_index(),
                    binding.texture(),
                    binding.role_semantics(),
                    binding.coordinate_source(),
                    binding.channel_mapping(),
                    binding.color_interpretation(),
                    binding.transform(),
                    binding.strength(),
                    binding.blend(),
                    binding.semantic_offset(),
                    binding.semantic_values(),
                    binding.source_key(),
                ),
            );
            mark_index(&mut reference_closure, binding.texture(), texture_count);
        }
    }

    {
        let geometry = cursor.geometry()?;
        while let Some(mesh) = geometry.next_logical_mesh()? {
            transcript.debug(
                "logical-mesh",
                &(
                    mesh.source_index(),
                    mesh.primitive_offset(),
                    mesh.primitive_count(),
                ),
            );
            mark_range(
                &mut reference_closure,
                mesh.primitive_offset(),
                mesh.primitive_count(),
                primitive_count,
            );
        }
        while let Some(mut primitive) = geometry.next_primitive()? {
            transcript.debug(
                "primitive",
                &(
                    primitive.source_index(),
                    primitive.topology(),
                    primitive.vertex_count(),
                    primitive.corner_count(),
                    primitive.face_count(),
                    primitive.polygon_boundaries_present(),
                    primitive.indices_present(),
                    primitive.index_count(),
                    primitive.material(),
                    primitive.attribute_offset(),
                    primitive.attribute_count(),
                ),
            );
            if let Some(boundaries) = primitive.take_polygon_boundaries() {
                let _ =
                    drain_geometry_u32(boundaries, &mut transcript, "primitive-polygon-boundary")?;
            }
            if let Some(indices) = primitive.take_indices() {
                let _ = drain_geometry_u32(indices, &mut transcript, "primitive-index")?;
            }
            mark_optional_index(&mut reference_closure, primitive.material(), material_count);
            mark_range(
                &mut reference_closure,
                primitive.attribute_offset(),
                primitive.attribute_count(),
                attribute_count,
            );
        }
        while let Some(attribute) = geometry.next_attribute()? {
            transcript.debug(
                "attribute",
                &(
                    attribute.source_index(),
                    attribute.semantic(),
                    attribute.domain(),
                    attribute.scalar_kind(),
                    attribute.lanes(),
                    attribute.normalization(),
                    attribute.payload_kind(),
                    attribute.element_count(),
                    attribute.explicit_element_count(),
                    attribute.canonical_scalar_count(),
                ),
            );
            match attribute.into_payload() {
                NodeSceneGeometryAttributePayload::DenseF32 { values } => {
                    let _ = drain_geometry_f32(values, &mut transcript, "attribute-value-f32")?;
                }
                NodeSceneGeometryAttributePayload::DenseU32 { values } => {
                    let _ = drain_geometry_u32(values, &mut transcript, "attribute-value-u32")?;
                }
                NodeSceneGeometryAttributePayload::DenseI32 { values } => {
                    let _ = drain_geometry_i32(values, &mut transcript, "attribute-value-i32")?;
                }
                NodeSceneGeometryAttributePayload::SparseF32 {
                    values,
                    indices,
                    default,
                } => {
                    let _ = drain_geometry_f32(values, &mut transcript, "attribute-value-f32")?;
                    let _ = drain_geometry_u32(indices, &mut transcript, "attribute-index")?;
                    let _ = drain_geometry_f32(default, &mut transcript, "attribute-default-f32")?;
                }
                NodeSceneGeometryAttributePayload::SparseU32 {
                    values,
                    indices,
                    default,
                } => {
                    let _ = drain_geometry_u32(values, &mut transcript, "attribute-value-u32")?;
                    let _ = drain_geometry_u32(indices, &mut transcript, "attribute-index")?;
                    let _ = drain_geometry_u32(default, &mut transcript, "attribute-default-u32")?;
                }
                NodeSceneGeometryAttributePayload::SparseI32 {
                    values,
                    indices,
                    default,
                } => {
                    let _ = drain_geometry_i32(values, &mut transcript, "attribute-value-i32")?;
                    let _ = drain_geometry_u32(indices, &mut transcript, "attribute-index")?;
                    let _ = drain_geometry_i32(default, &mut transcript, "attribute-default-i32")?;
                }
            }
        }
        while let Some(instance) = geometry.next_mesh_instance()? {
            transcript.debug(
                "mesh-instance",
                &(
                    instance.source_index(),
                    instance.owner_node(),
                    instance.logical_mesh(),
                ),
            );
            mark_index(&mut reference_closure, instance.owner_node(), node_count);
            mark_index(
                &mut reference_closure,
                instance.logical_mesh(),
                logical_mesh_count,
            );
        }
    }

    {
        let deformation = cursor.deformation()?;
        while let Some(mut skin) = deformation.next_skin()? {
            transcript.debug(
                "skin",
                &(
                    skin.source_index(),
                    skin.name(),
                    skin.skeleton_root(),
                    skin.matrix_spaces(),
                ),
            );
            mark_optional_index(&mut reference_closure, skin.skeleton_root(), node_count);
            if let Some(mut joints) = skin.take_joints() {
                while let Some(joint) = joints.next()? {
                    transcript.debug("skin-joint", &joint);
                    mark_index(&mut reference_closure, joint, node_count);
                    skin_joint_count = skin_joint_count
                        .checked_add(1)
                        .expect("facade skin-joint count must fit usize");
                }
                joints.finish()?;
            }
            if let Some(mut matrices) = skin.take_inverse_bind_matrices() {
                while let Some(matrix) = matrices.next()? {
                    transcript.debug("inverse-bind-matrix", &matrix);
                }
                matrices.finish()?;
            }
        }
        while let Some(mut logical_morph) = deformation.next_logical_morph()? {
            transcript.debug(
                "logical-morph",
                &(
                    logical_morph.source_index(),
                    logical_morph.definition_offset(),
                    logical_morph.definition_count(),
                ),
            );
            mark_range(
                &mut reference_closure,
                logical_morph.definition_offset(),
                logical_morph.definition_count(),
                morph_definition_count,
            );
            if let Some(values) = logical_morph.take_default_weights() {
                let _ = drain_deformation_f32(values, &mut transcript, "morph-default-weight")?;
            }
        }
        while let Some(definition) = deformation.next_morph_definition()? {
            transcript.debug(
                "morph-definition",
                &(
                    definition.source_index(),
                    definition.logical_mesh(),
                    definition.target_ordinal(),
                    definition.stable_identity(),
                    definition.name(),
                ),
            );
            mark_index(
                &mut reference_closure,
                definition.logical_mesh(),
                logical_mesh_count,
            );
        }
        while let Some(mut primitive) = deformation.next_primitive()? {
            transcript.debug(
                "deformation-primitive",
                &(
                    primitive.source_index(),
                    primitive.influences_present(),
                    primitive.morph_target_offset(),
                    primitive.morph_target_count(),
                ),
            );
            mark_range(
                &mut reference_closure,
                primitive.morph_target_offset(),
                primitive.morph_target_count(),
                morph_target_count,
            );
            if let Some(mut rows) = primitive.take_influences() {
                while let Some(row) = rows.next_row()? {
                    transcript.debug("influence-row", &row.vertex());
                    influence_row_count = influence_row_count
                        .checked_add(1)
                        .expect("facade influence-row count must fit usize");
                    let mut influences = row.into_influences();
                    while let Some(influence) = influences.next()? {
                        transcript.debug(
                            "influence",
                            &(
                                influence.palette_joint(),
                                influence.weight(),
                                influence.source_order(),
                            ),
                        );
                        influence_count = influence_count
                            .checked_add(1)
                            .expect("facade influence count must fit usize");
                    }
                    influences.finish()?;
                }
                rows.finish()?;
            }
        }
        while let Some(target) = deformation.next_morph_target()? {
            transcript.debug(
                "morph-target",
                &(
                    target.source_index(),
                    target.primitive(),
                    target.target_ordinal(),
                    target.delta_offset(),
                    target.delta_count(),
                ),
            );
            mark_index(&mut reference_closure, target.primitive(), primitive_count);
            mark_range(
                &mut reference_closure,
                target.delta_offset(),
                target.delta_count(),
                morph_delta_count,
            );
        }
        while let Some(delta) = deformation.next_morph_delta()? {
            transcript.debug(
                "morph-delta",
                &(delta.source_index(), delta.morph_target(), delta.semantic()),
            );
            mark_index(
                &mut reference_closure,
                delta.morph_target(),
                morph_target_count,
            );
            match delta.into_payload() {
                NodeSceneDeformationMorphPayload::Dense {
                    logical_count,
                    mut values,
                } => {
                    transcript.debug("morph-dense-logical-count", &logical_count);
                    while let Some(value) = values.next()? {
                        transcript.debug("morph-dense-value", &value);
                    }
                    values.finish()?;
                }
                NodeSceneDeformationMorphPayload::Sparse {
                    logical_count,
                    mut values,
                } => {
                    transcript.debug("morph-sparse-logical-count", &logical_count);
                    while let Some(value) = values.next()? {
                        transcript.debug("morph-sparse-value", &(value.index(), value.value()));
                    }
                    values.finish()?;
                }
            }
        }
        while let Some(mut instance) = deformation.next_mesh_instance()? {
            transcript.debug(
                "deformation-instance",
                &(
                    instance.source_index(),
                    instance.skin(),
                    instance.morph_weights_present(),
                ),
            );
            mark_optional_index(&mut reference_closure, instance.skin(), skin_count);
            if let Some(values) = instance.take_morph_weights() {
                let _ = drain_deformation_f32(values, &mut transcript, "instance-morph-weight")?;
            }
        }
    }

    {
        let animation = cursor.animation()?;
        while let Some(clip) = animation.next_clip()? {
            transcript.debug(
                "animation-clip",
                &(
                    clip.source_index(),
                    clip.duration(),
                    clip.time_domain(),
                    clip.channel_offset(),
                    clip.channel_count(),
                ),
            );
            mark_range(
                &mut reference_closure,
                clip.channel_offset(),
                clip.channel_count(),
                animation_channel_count,
            );
        }
        while let Some(channel) = animation.next_channel()? {
            transcript.debug(
                "animation-channel",
                &(
                    channel.source_index(),
                    channel.target(),
                    channel.interpolation(),
                    channel.pre_extrapolation(),
                    channel.post_extrapolation(),
                    channel.times(),
                    channel.values(),
                    channel.in_tangents(),
                    channel.out_tangents(),
                ),
            );
            animation_key_count = animation_key_count
                .checked_add(channel.key_count())
                .expect("facade animation-key count must fit usize");
            match channel.target() {
                NodeSceneAnimationTarget::NodeTranslation(node)
                | NodeSceneAnimationTarget::NodeRotation(node)
                | NodeSceneAnimationTarget::NodeScale(node) => {
                    reference_closure &= node > 0 && node <= node_count;
                }
                NodeSceneAnimationTarget::MeshInstanceMorphWeights(instance) => {
                    mark_index(&mut reference_closure, instance, mesh_instance_count);
                }
            }
        }
    }

    {
        let profile = cursor.profile()?;
        drain_profile_steps(
            || profile.step_resource(),
            &mut transcript,
            "profile-resource",
        )?;
        drain_profile_steps(|| profile.step_source(), &mut transcript, "profile-source")?;
        drain_profile_steps(
            || profile.step_provenance(),
            &mut transcript,
            "profile-provenance",
        )?;
        drain_profile_steps(
            || profile.step_evidence(),
            &mut transcript,
            "profile-evidence",
        )?;
        while let Some(owner) = profile.next_owner()? {
            transcript.debug("profile-owner", &owner);
        }
        while let Some(entry) = profile.next_metadata_entry()? {
            transcript.debug("profile-metadata", &entry);
        }
    }

    cursor.finish()?;
    reference_closure &= structural_reference_count == node_count;
    let (transcript_digest, transcript_bytes) = transcript.finish();
    Ok((
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
        },
        reference_closure,
        transcript_digest,
        transcript_bytes,
    ))
}

pub(in crate::canonical_scene::store) fn project_gltf_facade_observation_v1_for_probe(
    scene: &ValidatedNodeScene,
) -> FacadeObservationOutcomeV1 {
    let (totals, reference_closure_verified, transcript_digest, transcript_bytes) =
        match observe_facade_typed_view(scene) {
            Ok(observation) => observation,
            Err(error) => {
                return Err(FacadeObservationErrorV1 {
                    schema_id: FACADE_OBSERVATION_SCHEMA_ID,
                    error_class: "canonical-typed-view",
                    reason: error.reason(),
                });
            }
        };

    // The observer consumes every facade-intended typed domain and every nested payload cursor.
    // Its transcript is derived from those typed values; snapshot correlation is checked by the
    // independent snapshot phase and is never supplied by the caller.
    Ok(FacadeObservationV1 {
        schema_id: FACADE_OBSERVATION_SCHEMA_ID,
        transcript_digest,
        transcript_bytes,
        totals,
        reference_closure_verified,
        cursor_exhausted: true,
    })
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
    exhaust_profile_for_test(cursor.profile().expect("mixed-media profile cursor"));
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
    inspect: impl FnOnce(&NodeSceneConstructionRejection<StoreRuntimeError>),
) {
    let (prepared, rejection) =
        with_gltf_scene_input(GltfCapabilityScope::All, mutation, |input| {
            match SceneConstructionOperation::with_account_for_scene_input(
                account,
                test_limits(),
                input,
                SceneValidationLimits::default(),
            )
            .expect("invalid glTF shape operation admission")
            .finish_draft()
            .expect("invalid glTF shape rejection publication")
            {
                NodeSceneDraftBuild::Draft { .. } => {
                    panic!("invalid glTF shape must not produce a canonical draft")
                }
                NodeSceneDraftBuild::Failure {
                    authoring_rejection: Some(rejection),
                    prepared,
                } => (prepared, rejection),
                NodeSceneDraftBuild::Failure {
                    authoring_rejection: None,
                    ..
                } => panic!("invalid glTF shape must preserve its authoring rejection"),
            }
        });
    inspect(&rejection);
    let terminal = PreparedNodeSceneConstruction::from(prepared).handoff();
    assert!(matches!(
        &terminal,
        NodeSceneConstructionTerminal::RichFailure(_)
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
        GltfCapabilityMutation::PaletteJointOutOfRange,
        |failure| match failure {
            NodeSceneConstructionRejection::DeformationAuthoring(
                DeformationAuthoringFailure::Semantic(
                    DeformationAuthoringSemanticError::PaletteJointOutOfRange {
                        instance: 0,
                        logical_mesh: 0,
                        skin: 0,
                        joint_count: 2,
                        primitive: 0,
                        influence: 0,
                        palette_joint: 2,
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
            let build = SceneConstructionOperation::with_account_for_scene_input(
                &account,
                test_limits_with_work(work_limit),
                input,
                SceneValidationLimits::default(),
            )
            .expect("work-bounded glTF Scene input admission")
            .finish_draft()
            .expect("one-short glTF construction must publish a terminal outcome");

            let failure = rich_failure(
                build
                    .prepare_for_test()
                    .expect("one-short glTF work limit must prepare rich failure"),
            );
            let work = failure
                .ledger_evidence()
                .dimension_by_semantic(STORE_DIMENSION)
                .expect("failed glTF work evidence");
            assert_eq!(work.attempted(), successful_work);
            assert!(
                work.consumed() < work_limit,
                "the unexecuted canonical-validation suffix must remain unconsumed"
            );
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
            let build =
                SceneConstructionOperation::with_account_and_cancellation_for_scene_input_for_test(
                    &account,
                    test_limits(),
                    input,
                    SceneValidationLimits::default(),
                    cancellation.clone(),
                )
                .expect("cancellable glTF Scene input admission")
                .finish_draft()
                .expect("glTF input must activate before cancellation");
            let (draft, owner) = match build {
                NodeSceneDraftBuild::Draft { draft, owner } => (draft, owner),
                NodeSceneDraftBuild::Failure { .. } => {
                    panic!("glTF input must reach central validation before cancellation")
                }
            };

            cancellation.cancel();
            let failure = rich_failure(
                owner
                    .prepare_with_generation_source(draft, SceneGenerationId::reserve)
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
    let shape = inspect_gltf_capability_semantics_for_probe(&parts.scene);
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
