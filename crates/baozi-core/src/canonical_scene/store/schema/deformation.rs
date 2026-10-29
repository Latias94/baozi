//! Private canonical Scene deformation schema.

use baozi_operation::{RecordBlock, ScalarSequence, Utf8Block};

use crate::deform::{MorphDeltaSemantic, SkinMatrixSpace, SkinMatrixSpaces};

use super::CanonicalSchema;

pub(in super::super) const DEFORMATION_SCHEMA_REVISION: u32 = 2;
pub(in super::super) const DEFORMATION_DIRECTORY_FIELD_COUNT: usize = 39;
pub(in super::super) const DEFORMATION_SKIN_FIELD_COUNT: usize = 26;
pub(in super::super) const DEFORMATION_LOGICAL_MORPH_FIELD_COUNT: usize = 7;
pub(in super::super) const DEFORMATION_MORPH_DEFINITION_FIELD_COUNT: usize = 8;
pub(in super::super) const DEFORMATION_PRIMITIVE_FIELD_COUNT: usize = 13;
pub(in super::super) const DEFORMATION_MORPH_TARGET_FIELD_COUNT: usize = 6;
pub(in super::super) const DEFORMATION_MORPH_DELTA_FIELD_COUNT: usize = 12;
pub(in super::super) const DEFORMATION_MESH_INSTANCE_FIELD_COUNT: usize = 8;

const LOGICAL_U8_BYTES: u64 = 1;
const LOGICAL_U32_BYTES: u64 = 4;

/// Fixed scalar bytes, excluding optional concrete-store references and leaf payloads.
pub(in super::super) const DEFORMATION_DIRECTORY_FIXED_BYTES: u64 = 19 * LOGICAL_U32_BYTES;
pub(in super::super) const DEFORMATION_SKIN_FIXED_BYTES: u64 =
    (10 * LOGICAL_U32_BYTES) + (11 * LOGICAL_U8_BYTES);
pub(in super::super) const DEFORMATION_LOGICAL_MORPH_FIXED_BYTES: u64 = 5 * LOGICAL_U32_BYTES;
pub(in super::super) const DEFORMATION_MORPH_DEFINITION_FIXED_BYTES: u64 =
    (6 * LOGICAL_U32_BYTES) + LOGICAL_U8_BYTES;
pub(in super::super) const DEFORMATION_PRIMITIVE_FIXED_BYTES: u64 =
    (7 * LOGICAL_U32_BYTES) + LOGICAL_U8_BYTES;
pub(in super::super) const DEFORMATION_MORPH_TARGET_FIXED_BYTES: u64 = 5 * LOGICAL_U32_BYTES;
pub(in super::super) const DEFORMATION_MORPH_DELTA_FIXED_BYTES: u64 =
    (8 * LOGICAL_U32_BYTES) + (2 * LOGICAL_U8_BYTES);
pub(in super::super) const DEFORMATION_MESH_INSTANCE_FIXED_BYTES: u64 =
    (4 * LOGICAL_U32_BYTES) + (2 * LOGICAL_U8_BYTES);

pub(in super::super) const DEFORMATION_MORPH_POSITION: u8 = 0;
pub(in super::super) const DEFORMATION_MORPH_NORMAL: u8 = 1;
pub(in super::super) const DEFORMATION_MORPH_TANGENT: u8 = 2;
pub(in super::super) const DEFORMATION_MORPH_DENSE: u8 = 0;
pub(in super::super) const DEFORMATION_MORPH_SPARSE: u8 = 1;

pub(in super::super) const DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND: u8 = 0;
pub(in super::super) const DEFORMATION_SPACE_JOINT_LOCAL_BIND: u8 = 1;
pub(in super::super) const DEFORMATION_SPACE_JOINT_LOCAL: u8 = 2;
pub(in super::super) const DEFORMATION_SPACE_SCENE: u8 = 3;
pub(in super::super) const DEFORMATION_SPACE_MESH_INSTANCE_LOCAL: u8 = 4;

pub(in super::super) const fn encode_deformation_morph_semantic(
    semantic: MorphDeltaSemantic,
) -> u8 {
    match semantic {
        MorphDeltaSemantic::Position => DEFORMATION_MORPH_POSITION,
        MorphDeltaSemantic::Normal => DEFORMATION_MORPH_NORMAL,
        MorphDeltaSemantic::Tangent => DEFORMATION_MORPH_TANGENT,
    }
}

pub(in super::super) const fn encode_deformation_morph_payload(sparse: bool) -> u8 {
    if sparse {
        DEFORMATION_MORPH_SPARSE
    } else {
        DEFORMATION_MORPH_DENSE
    }
}

pub(in super::super) const fn encode_deformation_skin_space(space: SkinMatrixSpace) -> u8 {
    match space {
        SkinMatrixSpace::LogicalMeshLocalBind => DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND,
        SkinMatrixSpace::JointLocalBind => DEFORMATION_SPACE_JOINT_LOCAL_BIND,
        SkinMatrixSpace::JointLocal => DEFORMATION_SPACE_JOINT_LOCAL,
        SkinMatrixSpace::Scene => DEFORMATION_SPACE_SCENE,
        SkinMatrixSpace::MeshInstanceLocal => DEFORMATION_SPACE_MESH_INSTANCE_LOCAL,
    }
}

pub(in super::super) const fn encode_deformation_skin_matrix_spaces(
    spaces: SkinMatrixSpaces,
) -> [u8; 6] {
    [
        encode_deformation_skin_space(spaces.input_position()),
        encode_deformation_skin_space(spaces.inverse_bind_from()),
        encode_deformation_skin_space(spaces.inverse_bind_to()),
        encode_deformation_skin_space(spaces.current_joint_from()),
        encode_deformation_skin_space(spaces.current_joint_to()),
        encode_deformation_skin_space(spaces.output_position()),
    ]
}

pub(in super::super) mod deformation_directory_field {
    pub(crate) const SCHEMA_REVISION: usize = 0;
    pub(crate) const SKIN_COUNT: usize = 1;
    pub(crate) const SKINS: usize = 2;
    pub(crate) const JOINT_NODE_COUNT: usize = 3;
    pub(crate) const JOINT_NODES: usize = 4;
    pub(crate) const INVERSE_BIND_SCALAR_COUNT: usize = 5;
    pub(crate) const INVERSE_BIND_MATRICES: usize = 6;
    pub(crate) const LOGICAL_MORPH_COUNT: usize = 7;
    pub(crate) const LOGICAL_MORPHS: usize = 8;
    pub(crate) const PRIMITIVE_DEFORMATION_COUNT: usize = 9;
    pub(crate) const PRIMITIVE_DEFORMATIONS: usize = 10;
    pub(crate) const INFLUENCE_ROW_OFFSET_COUNT: usize = 11;
    pub(crate) const INFLUENCE_ROW_OFFSETS: usize = 12;
    pub(crate) const INFLUENCE_COUNT: usize = 13;
    pub(crate) const INFLUENCE_JOINTS: usize = 14;
    pub(crate) const INFLUENCE_WEIGHTS: usize = 15;
    pub(crate) const INFLUENCE_SOURCE_ORDERS: usize = 16;
    pub(crate) const MORPH_TARGET_COUNT: usize = 17;
    pub(crate) const MORPH_TARGETS: usize = 18;
    pub(crate) const MORPH_DELTA_COUNT: usize = 19;
    pub(crate) const MORPH_DELTAS: usize = 20;
    pub(crate) const MORPH_DEFAULT_COUNT: usize = 21;
    pub(crate) const MORPH_DEFAULTS: usize = 22;
    pub(crate) const MORPH_VALUE_COUNT: usize = 23;
    pub(crate) const MORPH_VALUES: usize = 24;
    pub(crate) const MORPH_SPARSE_INDEX_COUNT: usize = 25;
    pub(crate) const MORPH_SPARSE_INDICES: usize = 26;
    pub(crate) const MESH_INSTANCE_COUNT: usize = 27;
    pub(crate) const MESH_INSTANCES: usize = 28;
    pub(crate) const INSTANCE_MORPH_WEIGHT_COUNT: usize = 29;
    pub(crate) const INSTANCE_MORPH_WEIGHTS: usize = 30;
    pub(crate) const SKIN_CORRECTION_SCALAR_COUNT: usize = 31;
    pub(crate) const SKIN_CORRECTION_MATRICES: usize = 32;
    pub(crate) const MORPH_DEFINITION_COUNT: usize = 33;
    pub(crate) const MORPH_DEFINITIONS: usize = 34;
    pub(crate) const MORPH_NAME_BYTE_COUNT: usize = 35;
    pub(crate) const MORPH_NAMES: usize = 36;
    pub(crate) const SKIN_NAME_BYTE_COUNT: usize = 37;
    pub(crate) const SKIN_NAMES: usize = 38;
}

pub(in super::super) mod deformation_skin_field {
    pub(crate) const SKIN: usize = 0;
    pub(crate) const JOINT_OFFSET: usize = 1;
    pub(crate) const JOINT_COUNT: usize = 2;
    pub(crate) const JOINTS: usize = 3;
    pub(crate) const INVERSE_BIND_OFFSET: usize = 4;
    pub(crate) const INVERSE_BIND_COUNT: usize = 5;
    pub(crate) const INVERSE_BINDS: usize = 6;
    pub(crate) const SKELETON_ROOT_PRESENCE: usize = 7;
    pub(crate) const SKELETON_ROOT: usize = 8;
    pub(crate) const NODE_COUNT: usize = 9;
    pub(crate) const INPUT_POSITION_SPACE: usize = 10;
    pub(crate) const INVERSE_BIND_FROM_SPACE: usize = 11;
    pub(crate) const INVERSE_BIND_TO_SPACE: usize = 12;
    pub(crate) const CURRENT_JOINT_FROM_SPACE: usize = 13;
    pub(crate) const CURRENT_JOINT_TO_SPACE: usize = 14;
    pub(crate) const OUTPUT_POSITION_SPACE: usize = 15;
    pub(crate) const CORRECTION_PRESENCE: usize = 16;
    pub(crate) const CORRECTION_FROM_SPACE: usize = 17;
    pub(crate) const CORRECTION_TO_SPACE: usize = 18;
    pub(crate) const CORRECTION_MATRIX_OFFSET: usize = 19;
    pub(crate) const CORRECTION_MATRIX_COUNT: usize = 20;
    pub(crate) const CORRECTION_MATRICES: usize = 21;
    pub(crate) const NAME_PRESENCE: usize = 22;
    pub(crate) const NAME_OFFSET: usize = 23;
    pub(crate) const NAME_COUNT: usize = 24;
    pub(crate) const NAMES: usize = 25;
}

pub(in super::super) mod deformation_logical_morph_field {
    pub(crate) const LOGICAL_MESH: usize = 0;
    pub(crate) const DEFINITION_OFFSET: usize = 1;
    pub(crate) const DEFINITION_COUNT: usize = 2;
    pub(crate) const DEFINITIONS: usize = 3;
    pub(crate) const DEFAULT_WEIGHT_OFFSET: usize = 4;
    pub(crate) const DEFAULT_WEIGHT_COUNT: usize = 5;
    pub(crate) const DEFAULT_WEIGHTS: usize = 6;
}

pub(in super::super) mod deformation_morph_definition_field {
    pub(crate) const MORPH_DEFINITION: usize = 0;
    pub(crate) const LOGICAL_MESH: usize = 1;
    pub(crate) const TARGET_ORDINAL: usize = 2;
    pub(crate) const STABLE_IDENTITY: usize = 3;
    pub(crate) const NAME_PRESENCE: usize = 4;
    pub(crate) const NAME_OFFSET: usize = 5;
    pub(crate) const NAME_COUNT: usize = 6;
    pub(crate) const NAMES: usize = 7;
}

pub(in super::super) mod deformation_primitive_field {
    pub(crate) const PRIMITIVE: usize = 0;
    pub(crate) const INFLUENCE_PRESENCE: usize = 1;
    pub(crate) const ROW_OFFSET_OFFSET: usize = 2;
    pub(crate) const ROW_OFFSET_COUNT: usize = 3;
    pub(crate) const ROW_OFFSETS: usize = 4;
    pub(crate) const INFLUENCE_OFFSET: usize = 5;
    pub(crate) const INFLUENCE_COUNT: usize = 6;
    pub(crate) const INFLUENCE_JOINTS: usize = 7;
    pub(crate) const INFLUENCE_WEIGHTS: usize = 8;
    pub(crate) const INFLUENCE_SOURCE_ORDERS: usize = 9;
    pub(crate) const MORPH_TARGET_OFFSET: usize = 10;
    pub(crate) const MORPH_TARGET_COUNT: usize = 11;
    pub(crate) const MORPH_TARGETS: usize = 12;
}

pub(in super::super) mod deformation_morph_target_field {
    pub(crate) const MORPH_TARGET: usize = 0;
    pub(crate) const PRIMITIVE: usize = 1;
    pub(crate) const TARGET_ORDINAL: usize = 2;
    pub(crate) const DELTA_OFFSET: usize = 3;
    pub(crate) const DELTA_COUNT: usize = 4;
    pub(crate) const DELTAS: usize = 5;
}

pub(in super::super) mod deformation_morph_delta_field {
    pub(crate) const MORPH_DELTA: usize = 0;
    pub(crate) const MORPH_TARGET: usize = 1;
    pub(crate) const SEMANTIC: usize = 2;
    pub(crate) const PAYLOAD_KIND: usize = 3;
    pub(crate) const LOGICAL_COUNT: usize = 4;
    pub(crate) const EXPLICIT_COUNT: usize = 5;
    pub(crate) const VALUE_OFFSET: usize = 6;
    pub(crate) const VALUE_COUNT: usize = 7;
    pub(crate) const VALUES: usize = 8;
    pub(crate) const SPARSE_INDEX_OFFSET: usize = 9;
    pub(crate) const SPARSE_INDEX_COUNT: usize = 10;
    pub(crate) const SPARSE_INDICES: usize = 11;
}

pub(in super::super) mod deformation_mesh_instance_field {
    pub(crate) const MESH_INSTANCE: usize = 0;
    pub(crate) const SKIN_PRESENCE: usize = 1;
    pub(crate) const SKIN: usize = 2;
    pub(crate) const SKINS: usize = 3;
    pub(crate) const MORPH_WEIGHTS_PRESENCE: usize = 4;
    pub(crate) const MORPH_WEIGHT_OFFSET: usize = 5;
    pub(crate) const MORPH_WEIGHT_COUNT: usize = 6;
    pub(crate) const MORPH_WEIGHTS: usize = 7;
}

macro_rules! record_schemas {
    ($($name:ident),+ $(,)?) => {
        $(
            #[derive(Clone, Copy, PartialEq, Eq)]
            pub(in super::super) struct $name;

            impl CanonicalSchema for $name {
                type StoreKind = RecordBlock;
            }
        )+
    };
}

macro_rules! scalar_schemas {
    ($scalar:ty; $($name:ident),+ $(,)?) => {
        $(
            #[derive(Clone, Copy, PartialEq, Eq)]
            pub(in super::super) struct $name;

            impl CanonicalSchema for $name {
                type StoreKind = ScalarSequence<$scalar>;
            }
        )+
    };
}

record_schemas!(
    DeformationDirectorySchema,
    DeformationSkinTableSchema,
    DeformationLogicalMorphTableSchema,
    DeformationMorphDefinitionTableSchema,
    DeformationPrimitiveTableSchema,
    DeformationMorphTargetTableSchema,
    DeformationMorphDeltaTableSchema,
    DeformationMeshInstanceTableSchema,
);

scalar_schemas!(
    u32;
    DeformationSkinJointNodeSchema,
    DeformationInfluenceRowOffsetSchema,
    DeformationInfluenceJointSchema,
    DeformationInfluenceSourceOrderSchema,
    DeformationMorphSparseIndexSchema,
);

scalar_schemas!(
    f32;
    DeformationInverseBindMatrixSchema,
    DeformationSkinCorrectionMatrixSchema,
    DeformationInfluenceWeightSchema,
    DeformationMorphDefaultWeightSchema,
    DeformationMorphValueSchema,
    DeformationInstanceMorphWeightSchema,
);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct DeformationMorphNameSchema;

impl CanonicalSchema for DeformationMorphNameSchema {
    type StoreKind = Utf8Block;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct DeformationSkinNameSchema;

impl CanonicalSchema for DeformationSkinNameSchema {
    type StoreKind = Utf8Block;
}
