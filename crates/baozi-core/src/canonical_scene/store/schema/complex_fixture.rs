//! Test-only retained-store protocol stress fixture; not production Scene IR.

use super::CanonicalSchema;
use baozi_operation::{RecordBlock, ScalarSequence};

pub(in super::super) const COMPLEX_SCHEMA_REVISION: u32 = 1;
pub(in super::super) const COMPLEX_CANONICAL_BLOCK_COUNT: u32 = 26;
pub(in super::super) const COMPLEX_NODE_COUNT: u32 = 1;
pub(in super::super) const COMPLEX_DIRECTORY_FIELD_COUNT: usize = 31;
pub(in super::super) const COMPLEX_IMAGE_FIELD_COUNT: usize = 3;
pub(in super::super) const COMPLEX_SAMPLER_FIELD_COUNT: usize = 3;
pub(in super::super) const COMPLEX_TEXTURE_FIELD_COUNT: usize = 5;
pub(in super::super) const COMPLEX_MATERIAL_BINDING_FIELD_COUNT: usize = 3;
pub(in super::super) const COMPLEX_MATERIAL_FIELD_COUNT: usize = 3;
pub(in super::super) const COMPLEX_PRIMITIVE_FIELD_COUNT: usize = 7;
pub(in super::super) const COMPLEX_LOGICAL_MESH_FIELD_COUNT: usize = 3;
pub(in super::super) const COMPLEX_NODE_FIELD_COUNT: usize = 3;
pub(in super::super) const COMPLEX_MESH_INSTANCE_FIELD_COUNT: usize = 7;
pub(in super::super) const COMPLEX_SKIN_FIELD_COUNT: usize = 7;
pub(in super::super) const COMPLEX_JOINT_FIELD_COUNT: usize = 1;
pub(in super::super) const COMPLEX_INFLUENCE_PAYLOAD_FIELD_COUNT: usize = 4;
pub(in super::super) const COMPLEX_INFLUENCE_ROW_COUNT: usize = 3;
pub(in super::super) const COMPLEX_INVERSE_BIND_SCALAR_COUNT: usize = 16;
pub(in super::super) const COMPLEX_LOGICAL_MORPH_FIELD_COUNT: usize = 3;
pub(in super::super) const COMPLEX_INSTANCE_OVERRIDE_FIELD_COUNT: usize = 4;
pub(in super::super) const COMPLEX_ANIMATION_FIELD_COUNT: usize = 7;
pub(in super::super) const COMPLEX_ANIMATION_TIME_COUNT: usize = 2;
pub(in super::super) const COMPLEX_MORPH_WEIGHT_VALUE_WIDTH: usize = 1;
pub(in super::super) const COMPLEX_MORPH_WEIGHT_VALUE_COUNT: usize =
    COMPLEX_ANIMATION_TIME_COUNT * COMPLEX_MORPH_WEIGHT_VALUE_WIDTH;
pub(in super::super) const COMPLEX_METADATA_DESCRIPTOR_FIELD_COUNT: usize = 2;
pub(in super::super) const COMPLEX_PROVENANCE_ARENA_FIELD_COUNT: usize = 2;
pub(in super::super) const COMPLEX_SOURCE_ATTACHMENT_FIELD_COUNT: usize = 5;

pub(in super::super) const COMPLEX_IMAGE_MIME: &str = "image/png";
pub(in super::super) const COMPLEX_IMAGE_PAYLOAD: &[u8] = b"PNG\x0d\x0a";
pub(in super::super) const COMPLEX_SAMPLER_MAG_FILTER: u32 = 9729;
pub(in super::super) const COMPLEX_SAMPLER_MIN_FILTER: u32 = 9987;
pub(in super::super) const COMPLEX_METADATA: &str = "fixture-metadata:v1";
pub(in super::super) const COMPLEX_PROVENANCE: &[u8] = b"fixture-provenance:v1";
pub(in super::super) const COMPLEX_ANIMATION_TIMES: [f64; COMPLEX_ANIMATION_TIME_COUNT] =
    [0.0, 1.0];
pub(in super::super) const COMPLEX_MORPH_WEIGHT_VALUES: [f64; COMPLEX_MORPH_WEIGHT_VALUE_COUNT] =
    [0.25, 0.75];
pub(in super::super) const COMPLEX_INVERSE_BIND_MATRICES: [f32; COMPLEX_INVERSE_BIND_SCALAR_COUNT] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];
pub(in super::super) const COMPLEX_INFLUENCE_WEIGHTS: [f32; COMPLEX_INFLUENCE_ROW_COUNT] =
    [0.5, 0.3, 0.2];
pub(in super::super) const COMPLEX_LOGICAL_MORPH_WEIGHT: f64 = 0.75;
pub(in super::super) const COMPLEX_INSTANCE_OVERRIDE_WEIGHT: f64 = 0.25;

pub(in super::super) mod complex_directory_field {
    pub(crate) const SCHEMA_REVISION: usize = 0;
    pub(crate) const IMAGE_COUNT: usize = 1;
    pub(crate) const IMAGES: usize = 2;
    pub(crate) const SAMPLER_COUNT: usize = 3;
    pub(crate) const SAMPLERS: usize = 4;
    pub(crate) const TEXTURE_COUNT: usize = 5;
    pub(crate) const TEXTURES: usize = 6;
    pub(crate) const MATERIAL_BINDING_COUNT: usize = 7;
    pub(crate) const MATERIAL_BINDINGS: usize = 8;
    pub(crate) const MATERIAL_COUNT: usize = 9;
    pub(crate) const MATERIALS: usize = 10;
    pub(crate) const PRIMITIVE_COUNT: usize = 11;
    pub(crate) const PRIMITIVES: usize = 12;
    pub(crate) const LOGICAL_MESH_COUNT: usize = 13;
    pub(crate) const LOGICAL_MESHES: usize = 14;
    pub(crate) const NODE_COUNT: usize = 15;
    pub(crate) const NODES: usize = 16;
    pub(crate) const MESH_INSTANCE_COUNT: usize = 17;
    pub(crate) const MESH_INSTANCES: usize = 18;
    pub(crate) const SKIN_COUNT: usize = 19;
    pub(crate) const SKINS: usize = 20;
    pub(crate) const JOINT_COUNT: usize = 21;
    pub(crate) const JOINTS: usize = 22;
    pub(crate) const LOGICAL_MORPH_COUNT: usize = 23;
    pub(crate) const LOGICAL_MORPHS: usize = 24;
    pub(crate) const INSTANCE_OVERRIDE_COUNT: usize = 25;
    pub(crate) const INSTANCE_OVERRIDES: usize = 26;
    pub(crate) const ANIMATION_COUNT: usize = 27;
    pub(crate) const ANIMATIONS: usize = 28;
    pub(crate) const SOURCE_ATTACHMENT_COUNT: usize = 29;
    pub(crate) const SOURCE_ATTACHMENTS: usize = 30;
}

pub(in super::super) mod complex_image_field {
    pub(crate) const IMAGE: usize = 0;
    pub(crate) const MIME: usize = 1;
    pub(crate) const PAYLOAD: usize = 2;
}

pub(in super::super) mod complex_sampler_field {
    pub(crate) const SAMPLER: usize = 0;
    pub(crate) const MAG_FILTER: usize = 1;
    pub(crate) const MIN_FILTER: usize = 2;
}

pub(in super::super) mod complex_texture_field {
    pub(crate) const TEXTURE: usize = 0;
    pub(crate) const IMAGE_INDEX: usize = 1;
    pub(crate) const IMAGES: usize = 2;
    pub(crate) const SAMPLER_INDEX: usize = 3;
    pub(crate) const SAMPLERS: usize = 4;
}

pub(in super::super) mod complex_material_binding_field {
    pub(crate) const MATERIAL_BINDING: usize = 0;
    pub(crate) const TEXTURE_INDEX: usize = 1;
    pub(crate) const TEXTURES: usize = 2;
}

pub(in super::super) mod complex_material_field {
    pub(crate) const MATERIAL: usize = 0;
    pub(crate) const MATERIAL_BINDING_INDEX: usize = 1;
    pub(crate) const MATERIAL_BINDINGS: usize = 2;
}

pub(in super::super) mod complex_primitive_field {
    pub(crate) const PRIMITIVE: usize = 0;
    pub(crate) const MATERIAL_INDEX: usize = 1;
    pub(crate) const MATERIALS: usize = 2;
    pub(crate) const SKIN_INDEX: usize = 3;
    pub(crate) const SKINS: usize = 4;
    pub(crate) const INFLUENCE_ROW_COUNT: usize = 5;
    pub(crate) const INFLUENCES: usize = 6;
}

pub(in super::super) mod complex_logical_mesh_field {
    pub(crate) const LOGICAL_MESH: usize = 0;
    pub(crate) const PRIMITIVE_INDEX: usize = 1;
    pub(crate) const PRIMITIVES: usize = 2;
}

pub(in super::super) mod complex_node_field {
    pub(crate) const NODE: usize = 0;
    pub(crate) const MESH_INSTANCE_INDEX: usize = 1;
    pub(crate) const MESH_INSTANCES: usize = 2;
}

pub(in super::super) mod complex_mesh_instance_field {
    pub(crate) const MESH_INSTANCE: usize = 0;
    pub(crate) const LOGICAL_MESH_INDEX: usize = 1;
    pub(crate) const LOGICAL_MESHES: usize = 2;
    pub(crate) const SKIN_INDEX: usize = 3;
    pub(crate) const SKINS: usize = 4;
    pub(crate) const INSTANCE_OVERRIDE_INDEX: usize = 5;
    pub(crate) const INSTANCE_OVERRIDES: usize = 6;
}

pub(in super::super) mod complex_skin_field {
    pub(crate) const SKIN: usize = 0;
    pub(crate) const JOINT_INDEX: usize = 1;
    pub(crate) const JOINTS: usize = 2;
    pub(crate) const INVERSE_BIND_SCALAR_COUNT: usize = 3;
    pub(crate) const INVERSE_BINDS: usize = 4;
    pub(crate) const INFLUENCE_ROW_COUNT: usize = 5;
    pub(crate) const INFLUENCES: usize = 6;
}

pub(in super::super) mod complex_joint_field {
    pub(crate) const JOINT: usize = 0;
}

pub(in super::super) mod complex_influence_payload_field {
    pub(crate) const VERTEX: usize = 0;
    pub(crate) const JOINT_INDEX: usize = 1;
    pub(crate) const JOINTS: usize = 2;
    pub(crate) const WEIGHT: usize = 3;
}

pub(in super::super) mod complex_logical_morph_field {
    pub(crate) const LOGICAL_MORPH: usize = 0;
    pub(crate) const TARGET_COUNT: usize = 1;
    pub(crate) const DEFAULT_WEIGHT: usize = 2;
}

pub(in super::super) mod complex_instance_override_field {
    pub(crate) const INSTANCE_OVERRIDE: usize = 0;
    pub(crate) const LOGICAL_MORPH_INDEX: usize = 1;
    pub(crate) const LOGICAL_MORPHS: usize = 2;
    pub(crate) const WEIGHT: usize = 3;
}

pub(in super::super) mod complex_animation_field {
    pub(crate) const ANIMATION: usize = 0;
    pub(crate) const INSTANCE_OVERRIDE_INDEX: usize = 1;
    pub(crate) const INSTANCE_OVERRIDES: usize = 2;
    pub(crate) const KEYFRAME_COUNT: usize = 3;
    pub(crate) const TIMES: usize = 4;
    pub(crate) const MORPH_WEIGHT_VALUE_WIDTH: usize = 5;
    pub(crate) const MORPH_WEIGHT_VALUES: usize = 6;
}

pub(in super::super) mod complex_metadata_descriptor_field {
    pub(crate) const METADATA_DESCRIPTOR: usize = 0;
    pub(crate) const CONTENT: usize = 1;
}

pub(in super::super) mod complex_provenance_arena_field {
    pub(crate) const ARENA_REFERENCE: usize = 0;
    pub(crate) const CONTENT: usize = 1;
}

pub(in super::super) mod complex_source_attachment_field {
    pub(crate) const SOURCE_ATTACHMENT: usize = 0;
    pub(crate) const METADATA_DESCRIPTOR_INDEX: usize = 1;
    pub(crate) const METADATA_DESCRIPTORS: usize = 2;
    pub(crate) const PROVENANCE_ARENA_INDEX: usize = 3;
    pub(crate) const PROVENANCE_ARENAS: usize = 4;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexDirectorySchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexImageTableSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexImageMimeSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexImagePayloadSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexSamplerTableSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexTextureTableSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexMaterialBindingTableSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexMaterialTableSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexPrimitiveTableSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexLogicalMeshTableSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexNodeTableSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexMeshInstanceTableSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexSkinTableSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexJointTableSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexInverseBindMatricesSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexInfluencePayloadSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexLogicalMorphTableSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexInstanceOverrideTableSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexAnimationTableSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexAnimationTimesSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexMorphWeightValuesSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexMetadataSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexMetadataDescriptorSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexProvenanceSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexProvenanceArenaSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexSourceAttachmentSchema;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ComplexOrphanSchema;

impl CanonicalSchema for ComplexDirectorySchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexImageTableSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexImageMimeSchema {
    type StoreKind = baozi_operation::Utf8Block;
}
impl CanonicalSchema for ComplexImagePayloadSchema {
    type StoreKind = baozi_operation::ByteBlock;
}
impl CanonicalSchema for ComplexSamplerTableSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexTextureTableSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexMaterialBindingTableSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexMaterialTableSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexPrimitiveTableSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexLogicalMeshTableSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexNodeTableSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexMeshInstanceTableSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexSkinTableSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexJointTableSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexInverseBindMatricesSchema {
    type StoreKind = ScalarSequence<f32>;
}
impl CanonicalSchema for ComplexInfluencePayloadSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexLogicalMorphTableSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexInstanceOverrideTableSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexAnimationTableSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexAnimationTimesSchema {
    type StoreKind = ScalarSequence<f64>;
}
impl CanonicalSchema for ComplexMorphWeightValuesSchema {
    type StoreKind = ScalarSequence<f64>;
}
impl CanonicalSchema for ComplexMetadataSchema {
    type StoreKind = baozi_operation::Utf8Block;
}
impl CanonicalSchema for ComplexMetadataDescriptorSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexProvenanceSchema {
    type StoreKind = baozi_operation::ByteBlock;
}
impl CanonicalSchema for ComplexProvenanceArenaSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexSourceAttachmentSchema {
    type StoreKind = RecordBlock;
}
impl CanonicalSchema for ComplexOrphanSchema {
    type StoreKind = ScalarSequence<u8>;
}
