use std::marker::PhantomData;

use baozi_operation::{CanonicalStoreRef, RecordBlock, ScalarSequence};

pub(super) const SCHEMA_REVISION: u32 = 1;
pub(super) const NODE_COUNT: usize = 2;
pub(super) const REFERENCE_COUNT: usize = 1;
pub(super) const MATRIX_SCALAR_COUNT: usize = 32;
pub(super) const MATRIX_SCALARS_PER_NODE: usize = 16;
pub(super) const DIRECTORY_FIELD_COUNT: usize = 14;
pub(super) const NODE_FIELD_COUNT: usize = 7;
pub(super) const REFERENCE_FIELD_COUNT: usize = 3;
pub(super) const EVIDENCE_FIELD_COUNT: usize = 4;

pub(super) mod directory_field {
    pub(crate) const SCHEMA_REVISION: usize = 0;
    pub(crate) const ROOT_NODE: usize = 1;
    pub(crate) const NODE_COUNT: usize = 2;
    pub(crate) const NODES: usize = 3;
    pub(crate) const REFERENCE_COUNT: usize = 4;
    pub(crate) const REFERENCES: usize = 5;
    pub(crate) const ROOT_CHILDREN_PRESENCE: usize = 6;
    pub(crate) const ROOT_CHILDREN_COUNT: usize = 7;
    pub(crate) const CHILD_CHILDREN_PRESENCE: usize = 8;
    pub(crate) const CHILD_CHILDREN_COUNT: usize = 9;
    pub(crate) const METADATA_PRESENCE: usize = 10;
    pub(crate) const METADATA_COUNT: usize = 11;
    pub(crate) const MATRIX_SCALAR_COUNT: usize = 12;
    pub(crate) const MATRICES: usize = 13;
}

pub(super) mod node_field {
    pub(crate) const NODE: usize = 0;
    pub(crate) const PARENT_PRESENCE: usize = 1;
    pub(crate) const PARENT_REFERENCE_OFFSET: usize = 2;
    pub(crate) const PARENT_REFERENCE_COUNT: usize = 3;
    pub(crate) const MATRIX_OFFSET: usize = 4;
    pub(crate) const MATRIX_LENGTH: usize = 5;
    pub(crate) const MATRIX: usize = 6;
}

pub(super) mod reference_field {
    pub(crate) const REFERENCE: usize = 0;
    pub(crate) const PARENT: usize = 1;
    pub(crate) const CHILD: usize = 2;
}

pub(super) mod evidence_field {
    pub(crate) const SCHEMA_REVISION: usize = 0;
    pub(crate) const OUTCOME: usize = 1;
    pub(crate) const CANONICAL_BLOCK_COUNT: usize = 2;
    pub(crate) const NODE_COUNT: usize = 3;
}

pub(super) const PRESENCE_ABSENT: u8 = 0;
pub(super) const PRESENCE_PRESENT: u8 = 1;
pub(super) const EVIDENCE_OUTCOME_CANDIDATE: u32 = 1;
pub(super) const CANONICAL_BLOCK_COUNT: u32 = 4;

pub(super) trait CanonicalSchema: Copy + Send + Sync + 'static {
    type StoreKind;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct DirectorySchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct NodeTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ReferenceTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct AuthoredMatrixSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct OutcomeEvidenceSchema;

impl CanonicalSchema for DirectorySchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for NodeTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for ReferenceTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for AuthoredMatrixSchema {
    type StoreKind = ScalarSequence<f32>;
}

/// Non-owning core projection over one destination-qualified concrete-store slot.
pub(super) struct CanonicalBlockRef<Schema: CanonicalSchema> {
    raw: CanonicalStoreRef<Schema::StoreKind, Schema>,
    marker: PhantomData<fn() -> Schema>,
}

impl<Schema: CanonicalSchema> Copy for CanonicalBlockRef<Schema> {}

impl<Schema: CanonicalSchema> Clone for CanonicalBlockRef<Schema> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Schema: CanonicalSchema> PartialEq for CanonicalBlockRef<Schema> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<Schema: CanonicalSchema> Eq for CanonicalBlockRef<Schema> {}

impl<Schema: CanonicalSchema> std::fmt::Debug for CanonicalBlockRef<Schema> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CanonicalBlockRef(..)")
    }
}

impl<Schema: CanonicalSchema> CanonicalBlockRef<Schema> {
    pub(super) const fn from_store(raw: CanonicalStoreRef<Schema::StoreKind, Schema>) -> Self {
        Self {
            raw,
            marker: PhantomData,
        }
    }

    pub(super) const fn into_store(self) -> CanonicalStoreRef<Schema::StoreKind, Schema> {
        self.raw
    }
}

#[cfg(test)]
pub(super) const COMPLEX_SCHEMA_REVISION: u32 = 1;
#[cfg(test)]
pub(super) const COMPLEX_CANONICAL_BLOCK_COUNT: u32 = 26;
#[cfg(test)]
pub(super) const COMPLEX_NODE_COUNT: u32 = 1;
#[cfg(test)]
pub(super) const COMPLEX_DIRECTORY_FIELD_COUNT: usize = 31;
#[cfg(test)]
pub(super) const COMPLEX_IMAGE_FIELD_COUNT: usize = 3;
#[cfg(test)]
pub(super) const COMPLEX_SAMPLER_FIELD_COUNT: usize = 3;
#[cfg(test)]
pub(super) const COMPLEX_TEXTURE_FIELD_COUNT: usize = 5;
#[cfg(test)]
pub(super) const COMPLEX_MATERIAL_BINDING_FIELD_COUNT: usize = 3;
#[cfg(test)]
pub(super) const COMPLEX_MATERIAL_FIELD_COUNT: usize = 3;
#[cfg(test)]
pub(super) const COMPLEX_PRIMITIVE_FIELD_COUNT: usize = 7;
#[cfg(test)]
pub(super) const COMPLEX_LOGICAL_MESH_FIELD_COUNT: usize = 3;
#[cfg(test)]
pub(super) const COMPLEX_NODE_FIELD_COUNT: usize = 3;
#[cfg(test)]
pub(super) const COMPLEX_MESH_INSTANCE_FIELD_COUNT: usize = 7;
#[cfg(test)]
pub(super) const COMPLEX_SKIN_FIELD_COUNT: usize = 7;
#[cfg(test)]
pub(super) const COMPLEX_JOINT_FIELD_COUNT: usize = 1;
#[cfg(test)]
pub(super) const COMPLEX_INFLUENCE_PAYLOAD_FIELD_COUNT: usize = 4;
#[cfg(test)]
pub(super) const COMPLEX_INFLUENCE_ROW_COUNT: usize = 3;
#[cfg(test)]
pub(super) const COMPLEX_INVERSE_BIND_SCALAR_COUNT: usize = 16;
#[cfg(test)]
pub(super) const COMPLEX_LOGICAL_MORPH_FIELD_COUNT: usize = 3;
#[cfg(test)]
pub(super) const COMPLEX_INSTANCE_OVERRIDE_FIELD_COUNT: usize = 4;
#[cfg(test)]
pub(super) const COMPLEX_ANIMATION_FIELD_COUNT: usize = 7;
#[cfg(test)]
pub(super) const COMPLEX_ANIMATION_TIME_COUNT: usize = 2;
#[cfg(test)]
pub(super) const COMPLEX_MORPH_WEIGHT_VALUE_WIDTH: usize = 1;
#[cfg(test)]
pub(super) const COMPLEX_MORPH_WEIGHT_VALUE_COUNT: usize =
    COMPLEX_ANIMATION_TIME_COUNT * COMPLEX_MORPH_WEIGHT_VALUE_WIDTH;
#[cfg(test)]
pub(super) const COMPLEX_METADATA_DESCRIPTOR_FIELD_COUNT: usize = 2;
#[cfg(test)]
pub(super) const COMPLEX_PROVENANCE_ARENA_FIELD_COUNT: usize = 2;
#[cfg(test)]
pub(super) const COMPLEX_SOURCE_ATTACHMENT_FIELD_COUNT: usize = 5;

#[cfg(test)]
pub(super) const COMPLEX_IMAGE_MIME: &str = "image/png";
#[cfg(test)]
pub(super) const COMPLEX_IMAGE_PAYLOAD: &[u8] = b"PNG\x0d\x0a";
#[cfg(test)]
pub(super) const COMPLEX_SAMPLER_MAG_FILTER: u32 = 9729;
#[cfg(test)]
pub(super) const COMPLEX_SAMPLER_MIN_FILTER: u32 = 9987;
#[cfg(test)]
pub(super) const COMPLEX_METADATA: &str = "fixture-metadata:v1";
#[cfg(test)]
pub(super) const COMPLEX_PROVENANCE: &[u8] = b"fixture-provenance:v1";
#[cfg(test)]
pub(super) const COMPLEX_ANIMATION_TIMES: [f64; COMPLEX_ANIMATION_TIME_COUNT] = [0.0, 1.0];
#[cfg(test)]
pub(super) const COMPLEX_MORPH_WEIGHT_VALUES: [f64; COMPLEX_MORPH_WEIGHT_VALUE_COUNT] =
    [0.25, 0.75];
#[cfg(test)]
pub(super) const COMPLEX_INVERSE_BIND_MATRICES: [f32; COMPLEX_INVERSE_BIND_SCALAR_COUNT] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];
#[cfg(test)]
pub(super) const COMPLEX_INFLUENCE_WEIGHTS: [f32; COMPLEX_INFLUENCE_ROW_COUNT] = [0.5, 0.3, 0.2];
#[cfg(test)]
pub(super) const COMPLEX_LOGICAL_MORPH_WEIGHT: f64 = 0.75;
#[cfg(test)]
pub(super) const COMPLEX_INSTANCE_OVERRIDE_WEIGHT: f64 = 0.25;

#[cfg(test)]
pub(super) mod complex_directory_field {
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

#[cfg(test)]
pub(super) mod complex_image_field {
    pub(crate) const IMAGE: usize = 0;
    pub(crate) const MIME: usize = 1;
    pub(crate) const PAYLOAD: usize = 2;
}

#[cfg(test)]
pub(super) mod complex_sampler_field {
    pub(crate) const SAMPLER: usize = 0;
    pub(crate) const MAG_FILTER: usize = 1;
    pub(crate) const MIN_FILTER: usize = 2;
}

#[cfg(test)]
pub(super) mod complex_texture_field {
    pub(crate) const TEXTURE: usize = 0;
    pub(crate) const IMAGE_INDEX: usize = 1;
    pub(crate) const IMAGES: usize = 2;
    pub(crate) const SAMPLER_INDEX: usize = 3;
    pub(crate) const SAMPLERS: usize = 4;
}

#[cfg(test)]
pub(super) mod complex_material_binding_field {
    pub(crate) const MATERIAL_BINDING: usize = 0;
    pub(crate) const TEXTURE_INDEX: usize = 1;
    pub(crate) const TEXTURES: usize = 2;
}

#[cfg(test)]
pub(super) mod complex_material_field {
    pub(crate) const MATERIAL: usize = 0;
    pub(crate) const MATERIAL_BINDING_INDEX: usize = 1;
    pub(crate) const MATERIAL_BINDINGS: usize = 2;
}

#[cfg(test)]
pub(super) mod complex_primitive_field {
    pub(crate) const PRIMITIVE: usize = 0;
    pub(crate) const MATERIAL_INDEX: usize = 1;
    pub(crate) const MATERIALS: usize = 2;
    pub(crate) const SKIN_INDEX: usize = 3;
    pub(crate) const SKINS: usize = 4;
    pub(crate) const INFLUENCE_ROW_COUNT: usize = 5;
    pub(crate) const INFLUENCES: usize = 6;
}

#[cfg(test)]
pub(super) mod complex_logical_mesh_field {
    pub(crate) const LOGICAL_MESH: usize = 0;
    pub(crate) const PRIMITIVE_INDEX: usize = 1;
    pub(crate) const PRIMITIVES: usize = 2;
}

#[cfg(test)]
pub(super) mod complex_node_field {
    pub(crate) const NODE: usize = 0;
    pub(crate) const MESH_INSTANCE_INDEX: usize = 1;
    pub(crate) const MESH_INSTANCES: usize = 2;
}

#[cfg(test)]
pub(super) mod complex_mesh_instance_field {
    pub(crate) const MESH_INSTANCE: usize = 0;
    pub(crate) const LOGICAL_MESH_INDEX: usize = 1;
    pub(crate) const LOGICAL_MESHES: usize = 2;
    pub(crate) const SKIN_INDEX: usize = 3;
    pub(crate) const SKINS: usize = 4;
    pub(crate) const INSTANCE_OVERRIDE_INDEX: usize = 5;
    pub(crate) const INSTANCE_OVERRIDES: usize = 6;
}

#[cfg(test)]
pub(super) mod complex_skin_field {
    pub(crate) const SKIN: usize = 0;
    pub(crate) const JOINT_INDEX: usize = 1;
    pub(crate) const JOINTS: usize = 2;
    pub(crate) const INVERSE_BIND_SCALAR_COUNT: usize = 3;
    pub(crate) const INVERSE_BINDS: usize = 4;
    pub(crate) const INFLUENCE_ROW_COUNT: usize = 5;
    pub(crate) const INFLUENCES: usize = 6;
}

#[cfg(test)]
pub(super) mod complex_joint_field {
    pub(crate) const JOINT: usize = 0;
}

#[cfg(test)]
pub(super) mod complex_influence_payload_field {
    pub(crate) const VERTEX: usize = 0;
    pub(crate) const JOINT_INDEX: usize = 1;
    pub(crate) const JOINTS: usize = 2;
    pub(crate) const WEIGHT: usize = 3;
}

#[cfg(test)]
pub(super) mod complex_logical_morph_field {
    pub(crate) const LOGICAL_MORPH: usize = 0;
    pub(crate) const TARGET_COUNT: usize = 1;
    pub(crate) const DEFAULT_WEIGHT: usize = 2;
}

#[cfg(test)]
pub(super) mod complex_instance_override_field {
    pub(crate) const INSTANCE_OVERRIDE: usize = 0;
    pub(crate) const LOGICAL_MORPH_INDEX: usize = 1;
    pub(crate) const LOGICAL_MORPHS: usize = 2;
    pub(crate) const WEIGHT: usize = 3;
}

#[cfg(test)]
pub(super) mod complex_animation_field {
    pub(crate) const ANIMATION: usize = 0;
    pub(crate) const INSTANCE_OVERRIDE_INDEX: usize = 1;
    pub(crate) const INSTANCE_OVERRIDES: usize = 2;
    pub(crate) const KEYFRAME_COUNT: usize = 3;
    pub(crate) const TIMES: usize = 4;
    pub(crate) const MORPH_WEIGHT_VALUE_WIDTH: usize = 5;
    pub(crate) const MORPH_WEIGHT_VALUES: usize = 6;
}

#[cfg(test)]
pub(super) mod complex_metadata_descriptor_field {
    pub(crate) const METADATA_DESCRIPTOR: usize = 0;
    pub(crate) const CONTENT: usize = 1;
}

#[cfg(test)]
pub(super) mod complex_provenance_arena_field {
    pub(crate) const ARENA_REFERENCE: usize = 0;
    pub(crate) const CONTENT: usize = 1;
}

#[cfg(test)]
pub(super) mod complex_source_attachment_field {
    pub(crate) const SOURCE_ATTACHMENT: usize = 0;
    pub(crate) const METADATA_DESCRIPTOR_INDEX: usize = 1;
    pub(crate) const METADATA_DESCRIPTORS: usize = 2;
    pub(crate) const PROVENANCE_ARENA_INDEX: usize = 3;
    pub(crate) const PROVENANCE_ARENAS: usize = 4;
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexDirectorySchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexImageTableSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexImageMimeSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexImagePayloadSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexSamplerTableSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexTextureTableSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexMaterialBindingTableSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexMaterialTableSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexPrimitiveTableSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexLogicalMeshTableSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexNodeTableSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexMeshInstanceTableSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexSkinTableSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexJointTableSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexInverseBindMatricesSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexInfluencePayloadSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexLogicalMorphTableSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexInstanceOverrideTableSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexAnimationTableSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexAnimationTimesSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexMorphWeightValuesSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexMetadataSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexMetadataDescriptorSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexProvenanceSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexProvenanceArenaSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexSourceAttachmentSchema;
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ComplexOrphanSchema;

#[cfg(test)]
impl CanonicalSchema for ComplexDirectorySchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexImageTableSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexImageMimeSchema {
    type StoreKind = baozi_operation::Utf8Block;
}
#[cfg(test)]
impl CanonicalSchema for ComplexImagePayloadSchema {
    type StoreKind = baozi_operation::ByteBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexSamplerTableSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexTextureTableSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexMaterialBindingTableSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexMaterialTableSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexPrimitiveTableSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexLogicalMeshTableSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexNodeTableSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexMeshInstanceTableSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexSkinTableSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexJointTableSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexInverseBindMatricesSchema {
    type StoreKind = ScalarSequence<f32>;
}
#[cfg(test)]
impl CanonicalSchema for ComplexInfluencePayloadSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexLogicalMorphTableSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexInstanceOverrideTableSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexAnimationTableSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexAnimationTimesSchema {
    type StoreKind = ScalarSequence<f64>;
}
#[cfg(test)]
impl CanonicalSchema for ComplexMorphWeightValuesSchema {
    type StoreKind = ScalarSequence<f64>;
}
#[cfg(test)]
impl CanonicalSchema for ComplexMetadataSchema {
    type StoreKind = baozi_operation::Utf8Block;
}
#[cfg(test)]
impl CanonicalSchema for ComplexMetadataDescriptorSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexProvenanceSchema {
    type StoreKind = baozi_operation::ByteBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexProvenanceArenaSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexSourceAttachmentSchema {
    type StoreKind = RecordBlock;
}
#[cfg(test)]
impl CanonicalSchema for ComplexOrphanSchema {
    type StoreKind = ScalarSequence<u8>;
}
