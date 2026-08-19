use baozi_operation::__retained_store_integration::activation::{
    CanonicalValidationMember, CanonicalValidationSession,
};
use baozi_operation::{
    CanonicalRecordRef, CanonicalRecordView, CanonicalSequenceRef, CanonicalUtf8Ref,
    OperationOwner, ProgressPhase, ProgressUnit, RecordBlock, ScalarSequence, SequenceView,
    StoreValidationSession, StoreWorkError, StoredScalar, Utf8Block,
};

use crate::canonical_scene::store::production::{
    DeformationMorphPayloadKind, MorphDeltaPayloadAuthoring, Presence, ValidatedDeformationBatch,
    ValidatedNodeBatch,
};
use crate::canonical_scene::store::schema::deformation::{
    DEFORMATION_DIRECTORY_FIELD_COUNT, DEFORMATION_LOGICAL_MORPH_FIELD_COUNT,
    DEFORMATION_MESH_INSTANCE_FIELD_COUNT, DEFORMATION_MORPH_DEFINITION_FIELD_COUNT,
    DEFORMATION_MORPH_DELTA_FIELD_COUNT, DEFORMATION_MORPH_DENSE, DEFORMATION_MORPH_NORMAL,
    DEFORMATION_MORPH_POSITION, DEFORMATION_MORPH_SPARSE, DEFORMATION_MORPH_TANGENT,
    DEFORMATION_MORPH_TARGET_FIELD_COUNT, DEFORMATION_PRIMITIVE_FIELD_COUNT,
    DEFORMATION_SCHEMA_REVISION, DEFORMATION_SKIN_FIELD_COUNT, DEFORMATION_SPACE_JOINT_LOCAL,
    DEFORMATION_SPACE_JOINT_LOCAL_BIND, DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND,
    DEFORMATION_SPACE_MESH_INSTANCE_LOCAL, DEFORMATION_SPACE_SCENE, DeformationDirectorySchema,
    DeformationInfluenceJointSchema, DeformationInfluenceRowOffsetSchema,
    DeformationInfluenceSourceOrderSchema, DeformationInfluenceWeightSchema,
    DeformationInstanceMorphWeightSchema, DeformationInverseBindMatrixSchema,
    DeformationLogicalMorphTableSchema, DeformationMeshInstanceTableSchema,
    DeformationMorphDefaultWeightSchema, DeformationMorphDefinitionTableSchema,
    DeformationMorphDeltaTableSchema, DeformationMorphNameSchema,
    DeformationMorphSparseIndexSchema, DeformationMorphTargetTableSchema,
    DeformationMorphValueSchema, DeformationPrimitiveTableSchema,
    DeformationSkinCorrectionMatrixSchema, DeformationSkinJointNodeSchema,
    DeformationSkinNameSchema, DeformationSkinTableSchema, deformation_directory_field,
    deformation_logical_morph_field, deformation_mesh_instance_field,
    deformation_morph_definition_field, deformation_morph_delta_field,
    deformation_morph_target_field, deformation_primitive_field, deformation_skin_field,
    encode_deformation_morph_payload, encode_deformation_morph_semantic,
    encode_deformation_skin_matrix_spaces, encode_deformation_skin_space,
};
use crate::canonical_scene::store::schema::geometry::{
    GEOMETRY_LOGICAL_MESH_FIELD_COUNT, GEOMETRY_MESH_INSTANCE_FIELD_COUNT,
    GEOMETRY_PRIMITIVE_FIELD_COUNT, GeometryLogicalMeshTableSchema,
    GeometryMeshInstanceTableSchema, GeometryPrimitiveTableSchema, geometry_logical_mesh_field,
    geometry_mesh_instance_field, geometry_primitive_field,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, NODE_SCENE_NODE_FIELD_COUNT, NodeSceneNodeTableSchema, PRESENCE_ABSENT,
    PRESENCE_PRESENT,
};
use crate::entity::MAX_ENTITY_NAME_BYTES;

use super::super::require_scalar;
use super::geometry::{
    GeometryLogicalMeshValidationProjection, GeometryMeshInstanceValidationProjection,
    GeometryPrimitiveValidationProjection,
};

const MATRIX_SCALAR_COUNT: u32 = 16;
const MORPH_VECTOR_WIDTH: u32 = 3;

struct FreshDeformationEquivalence<'fresh, 'node, 'deformation, R, NR> {
    nodes: &'fresh ValidatedNodeBatch<'node, NR>,
    deformation: &'fresh ValidatedDeformationBatch<'deformation, R>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StoredPresence {
    Absent,
    Present,
}

struct DeformationUniquenessScratch<'scratch> {
    marks: &'scratch mut [u64],
    next_marker: u64,
}

impl<'scratch> DeformationUniquenessScratch<'scratch> {
    /// Reuses one centrally zeroed buffer by assigning each dense group a distinct marker.
    const fn new(marks: &'scratch mut [u64]) -> Self {
        Self {
            marks,
            next_marker: 1,
        }
    }

    fn begin_group(&mut self, width: usize) -> Option<DeformationUniquenessGroup<'_>> {
        let marker = self.next_marker;
        self.next_marker = self.next_marker.checked_add(1)?;
        Some(DeformationUniquenessGroup {
            marks: self.marks.get_mut(..width)?,
            marker,
        })
    }
}

struct DeformationUniquenessGroup<'scratch> {
    marks: &'scratch mut [u64],
    marker: u64,
}

impl DeformationUniquenessGroup<'_> {
    fn mark(&mut self, ordinal: u32) -> Option<()> {
        let mark = self.marks.get_mut(usize::try_from(ordinal).ok()?)?;
        if *mark == self.marker {
            return None;
        }
        *mark = self.marker;
        Some(())
    }
}

#[derive(Clone, Copy)]
struct DeformationDependencyViews<'storage> {
    nodes: CanonicalRecordView<'storage>,
    logical_meshes: Option<CanonicalRecordView<'storage>>,
    primitives: Option<CanonicalRecordView<'storage>>,
    mesh_instances: Option<CanonicalRecordView<'storage>>,
}

#[derive(Clone, Copy)]
struct DeformationPrimitiveViews<'storage> {
    primitives: Option<CanonicalRecordView<'storage>>,
    influence_joints: Option<SequenceView<'storage, u32>>,
    maximum_uniqueness_count: usize,
}

#[derive(Clone, Copy)]
pub(super) struct DeformationValidationDependencies {
    nodes: CanonicalBlockRef<NodeSceneNodeTableSchema>,
    logical_meshes: Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>>,
    primitives: Option<CanonicalBlockRef<GeometryPrimitiveTableSchema>>,
    mesh_instances: Option<CanonicalBlockRef<GeometryMeshInstanceTableSchema>>,
}

impl DeformationValidationDependencies {
    pub(super) const fn new(
        nodes: CanonicalBlockRef<NodeSceneNodeTableSchema>,
        logical_meshes: Option<CanonicalBlockRef<GeometryLogicalMeshTableSchema>>,
        primitives: Option<CanonicalBlockRef<GeometryPrimitiveTableSchema>>,
        mesh_instances: Option<CanonicalBlockRef<GeometryMeshInstanceTableSchema>>,
    ) -> Self {
        Self {
            nodes,
            logical_meshes,
            primitives,
            mesh_instances,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct DeformationValidatedRefs {
    pub(super) skins: Option<CanonicalBlockRef<DeformationSkinTableSchema>>,
    pub(super) logical_morphs: Option<CanonicalBlockRef<DeformationLogicalMorphTableSchema>>,
    pub(super) morph_definitions: Option<CanonicalBlockRef<DeformationMorphDefinitionTableSchema>>,
    pub(super) primitives: Option<CanonicalBlockRef<DeformationPrimitiveTableSchema>>,
    pub(super) morph_targets: Option<CanonicalBlockRef<DeformationMorphTargetTableSchema>>,
    pub(super) morph_deltas: Option<CanonicalBlockRef<DeformationMorphDeltaTableSchema>>,
    pub(super) mesh_instances: Option<CanonicalBlockRef<DeformationMeshInstanceTableSchema>>,
    pub(super) joint_nodes: Option<CanonicalBlockRef<DeformationSkinJointNodeSchema>>,
    pub(super) inverse_bind_matrices: Option<CanonicalBlockRef<DeformationInverseBindMatrixSchema>>,
    pub(super) skin_correction_matrices:
        Option<CanonicalBlockRef<DeformationSkinCorrectionMatrixSchema>>,
    pub(super) skin_names: Option<CanonicalBlockRef<DeformationSkinNameSchema>>,
    pub(super) influence_row_offsets:
        Option<CanonicalBlockRef<DeformationInfluenceRowOffsetSchema>>,
    pub(super) influence_joints: Option<CanonicalBlockRef<DeformationInfluenceJointSchema>>,
    pub(super) influence_weights: Option<CanonicalBlockRef<DeformationInfluenceWeightSchema>>,
    pub(super) influence_source_orders:
        Option<CanonicalBlockRef<DeformationInfluenceSourceOrderSchema>>,
    pub(super) morph_defaults: Option<CanonicalBlockRef<DeformationMorphDefaultWeightSchema>>,
    pub(super) morph_names: Option<CanonicalBlockRef<DeformationMorphNameSchema>>,
    pub(super) morph_values: Option<CanonicalBlockRef<DeformationMorphValueSchema>>,
    pub(super) morph_sparse_indices: Option<CanonicalBlockRef<DeformationMorphSparseIndexSchema>>,
    pub(super) instance_morph_weights:
        Option<CanonicalBlockRef<DeformationInstanceMorphWeightSchema>>,
}

#[derive(Clone, Copy)]
pub(super) struct DeformationValidatedShape {
    pub(super) refs: DeformationValidatedRefs,
    pub(super) payload_block_count: u32,
    pub(super) skin_count: u32,
    pub(super) logical_morph_count: u32,
    pub(super) morph_definition_count: u32,
    pub(super) primitive_count: u32,
    pub(super) morph_target_count: u32,
    pub(super) morph_delta_count: u32,
    pub(super) mesh_instance_count: u32,
    pub(super) maximum_uniqueness_count: usize,
}

/// Validation-scoped deformation members proven by the typed central-validation lane.
#[derive(Clone, Copy)]
pub(super) struct DeformationValidatedMembers<'session> {
    pub(super) directory:
        CanonicalValidationMember<'session, RecordBlock, DeformationDirectorySchema>,
    pub(super) skins:
        Option<CanonicalValidationMember<'session, RecordBlock, DeformationSkinTableSchema>>,
    pub(super) logical_morphs: Option<
        CanonicalValidationMember<'session, RecordBlock, DeformationLogicalMorphTableSchema>,
    >,
    pub(super) morph_definitions: Option<
        CanonicalValidationMember<'session, RecordBlock, DeformationMorphDefinitionTableSchema>,
    >,
    pub(super) primitives:
        Option<CanonicalValidationMember<'session, RecordBlock, DeformationPrimitiveTableSchema>>,
    pub(super) morph_targets:
        Option<CanonicalValidationMember<'session, RecordBlock, DeformationMorphTargetTableSchema>>,
    pub(super) morph_deltas:
        Option<CanonicalValidationMember<'session, RecordBlock, DeformationMorphDeltaTableSchema>>,
    pub(super) mesh_instances: Option<
        CanonicalValidationMember<'session, RecordBlock, DeformationMeshInstanceTableSchema>,
    >,
    pub(super) joint_nodes: Option<
        CanonicalValidationMember<'session, ScalarSequence<u32>, DeformationSkinJointNodeSchema>,
    >,
    pub(super) inverse_bind_matrices: Option<
        CanonicalValidationMember<
            'session,
            ScalarSequence<f32>,
            DeformationInverseBindMatrixSchema,
        >,
    >,
    pub(super) skin_correction_matrices: Option<
        CanonicalValidationMember<
            'session,
            ScalarSequence<f32>,
            DeformationSkinCorrectionMatrixSchema,
        >,
    >,
    pub(super) skin_names:
        Option<CanonicalValidationMember<'session, Utf8Block, DeformationSkinNameSchema>>,
    pub(super) influence_row_offsets: Option<
        CanonicalValidationMember<
            'session,
            ScalarSequence<u32>,
            DeformationInfluenceRowOffsetSchema,
        >,
    >,
    pub(super) influence_joints: Option<
        CanonicalValidationMember<'session, ScalarSequence<u32>, DeformationInfluenceJointSchema>,
    >,
    pub(super) influence_weights: Option<
        CanonicalValidationMember<'session, ScalarSequence<f32>, DeformationInfluenceWeightSchema>,
    >,
    pub(super) influence_source_orders: Option<
        CanonicalValidationMember<
            'session,
            ScalarSequence<u32>,
            DeformationInfluenceSourceOrderSchema,
        >,
    >,
    pub(super) morph_defaults: Option<
        CanonicalValidationMember<
            'session,
            ScalarSequence<f32>,
            DeformationMorphDefaultWeightSchema,
        >,
    >,
    pub(super) morph_names:
        Option<CanonicalValidationMember<'session, Utf8Block, DeformationMorphNameSchema>>,
    pub(super) morph_values: Option<
        CanonicalValidationMember<'session, ScalarSequence<f32>, DeformationMorphValueSchema>,
    >,
    pub(super) morph_sparse_indices: Option<
        CanonicalValidationMember<'session, ScalarSequence<u32>, DeformationMorphSparseIndexSchema>,
    >,
    pub(super) instance_morph_weights: Option<
        CanonicalValidationMember<
            'session,
            ScalarSequence<f32>,
            DeformationInstanceMorphWeightSchema,
        >,
    >,
}

/// Typed deformation validation result retained only for the enclosing validation session.
#[derive(Clone, Copy)]
pub(super) struct DeformationValidatedTypedShape<'session> {
    pub(super) members: DeformationValidatedMembers<'session>,
    pub(super) payload_block_count: u32,
    pub(super) skin_count: u32,
    pub(super) logical_morph_count: u32,
    pub(super) morph_definition_count: u32,
    pub(super) primitive_count: u32,
    pub(super) morph_target_count: u32,
    pub(super) morph_delta_count: u32,
    pub(super) mesh_instance_count: u32,
    pub(super) maximum_uniqueness_count: usize,
}

#[derive(Clone, Copy)]
pub(super) struct DeformationGeometryValidationProjection<'projection> {
    pub(super) logical_meshes: &'projection [GeometryLogicalMeshValidationProjection],
    pub(super) primitives: &'projection [GeometryPrimitiveValidationProjection],
    pub(super) mesh_instances: &'projection [GeometryMeshInstanceValidationProjection],
}

#[derive(Clone, Copy)]
pub(super) struct DeformationValidationDependenciesMetered<'session, 'projection> {
    pub(super) nodes: CanonicalValidationMember<'session, RecordBlock, NodeSceneNodeTableSchema>,
    pub(super) node_count: u32,
    pub(super) logical_meshes:
        Option<CanonicalValidationMember<'session, RecordBlock, GeometryLogicalMeshTableSchema>>,
    pub(super) primitives:
        Option<CanonicalValidationMember<'session, RecordBlock, GeometryPrimitiveTableSchema>>,
    pub(super) mesh_instances:
        Option<CanonicalValidationMember<'session, RecordBlock, GeometryMeshInstanceTableSchema>>,
    pub(super) geometry: DeformationGeometryValidationProjection<'projection>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct DeformationSkinValidationProjection {
    joint_count: u32,
    name_present: bool,
    name_offset: u32,
    name_count: u32,
}

impl DeformationSkinValidationProjection {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        joint_count: 0,
        name_present: false,
        name_offset: 0,
        name_count: 0,
    };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct DeformationMorphDefinitionValidationProjection {
    logical_mesh: u32,
    target_ordinal: u32,
    stable_identity: u32,
    name_present: bool,
    name_offset: u32,
    name_count: u32,
}

impl DeformationMorphDefinitionValidationProjection {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        logical_mesh: 0,
        target_ordinal: 0,
        stable_identity: 0,
        name_present: false,
        name_offset: 0,
        name_count: 0,
    };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct DeformationLogicalMorphValidationProjection {
    definition_offset: u32,
    definition_count: u32,
    default_weight_offset: u32,
    default_weight_count: u32,
}

impl DeformationLogicalMorphValidationProjection {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        definition_offset: 0,
        definition_count: 0,
        default_weight_offset: 0,
        default_weight_count: 0,
    };

    pub(in crate::canonical_scene::store) const fn definition_count(self) -> u32 {
        self.definition_count
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct DeformationMorphDeltaValidationProjection {
    logical_count: u32,
    sparse_index_offset: u32,
    sparse_index_count: u32,
}

impl DeformationMorphDeltaValidationProjection {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        logical_count: 0,
        sparse_index_offset: 0,
        sparse_index_count: 0,
    };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct DeformationMorphTargetValidationProjection {
    primitive: u32,
    delta_offset: u32,
    delta_count: u32,
}

impl DeformationMorphTargetValidationProjection {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        primitive: 0,
        delta_offset: 0,
        delta_count: 0,
    };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct DeformationPrimitiveValidationProjection {
    influences_present: bool,
    row_offset_offset: u32,
    row_offset_count: u32,
    influence_offset: u32,
    influence_count: u32,
    morph_target_offset: u32,
    morph_target_count: u32,
}

impl DeformationPrimitiveValidationProjection {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        influences_present: false,
        row_offset_offset: 0,
        row_offset_count: 0,
        influence_offset: 0,
        influence_count: 0,
        morph_target_offset: 0,
        morph_target_count: 0,
    };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct DeformationMeshInstanceValidationProjection {
    skin_present: bool,
    skin: u32,
}

impl DeformationMeshInstanceValidationProjection {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        skin_present: false,
        skin: 0,
    };
}

/// Exact caller-provided deformation projection capacity derived from the sealed Scene plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct DeformationValidationScratchRequirements {
    pub(in crate::canonical_scene::store) skins: usize,
    pub(in crate::canonical_scene::store) morph_definitions: usize,
    pub(in crate::canonical_scene::store) logical_morphs: usize,
    pub(in crate::canonical_scene::store) morph_deltas: usize,
    pub(in crate::canonical_scene::store) morph_targets: usize,
    pub(in crate::canonical_scene::store) primitives: usize,
    pub(in crate::canonical_scene::store) mesh_instances: usize,
    pub(in crate::canonical_scene::store) influence_row_offsets: usize,
    pub(in crate::canonical_scene::store) influence_joints: usize,
    pub(in crate::canonical_scene::store) influence_source_orders: usize,
}

impl DeformationValidationScratchRequirements {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::canonical_scene::store) fn try_new(
        skin_count: u32,
        morph_definition_count: u32,
        logical_morph_count: u32,
        morph_delta_count: u32,
        morph_target_count: u32,
        primitive_count: u32,
        mesh_instance_count: u32,
        influence_row_offset_count: u32,
        influence_count: u32,
    ) -> Option<Self> {
        Some(Self {
            skins: usize::try_from(skin_count).ok()?,
            morph_definitions: usize::try_from(morph_definition_count).ok()?,
            logical_morphs: usize::try_from(logical_morph_count).ok()?,
            morph_deltas: usize::try_from(morph_delta_count).ok()?,
            morph_targets: usize::try_from(morph_target_count).ok()?,
            primitives: usize::try_from(primitive_count).ok()?,
            mesh_instances: usize::try_from(mesh_instance_count).ok()?,
            influence_row_offsets: usize::try_from(influence_row_offset_count).ok()?,
            influence_joints: usize::try_from(influence_count).ok()?,
            influence_source_orders: usize::try_from(influence_count).ok()?,
        })
    }

    pub(in crate::canonical_scene::store) fn initialization_work_units(self) -> Option<u64> {
        [
            self.skins,
            self.morph_definitions,
            self.logical_morphs,
            self.morph_deltas,
            self.morph_targets,
            self.primitives,
            self.mesh_instances,
            self.influence_row_offsets,
            self.influence_joints,
            self.influence_source_orders,
        ]
        .into_iter()
        .try_fold(0_u64, |total, count| {
            total.checked_add(u64::try_from(count).ok()?)
        })
    }
}

/// Borrowed projection storage initialized by the enclosing typed validation scratch owner.
pub(in crate::canonical_scene::store) struct DeformationValidationScratch<'scratch> {
    skins: &'scratch mut [DeformationSkinValidationProjection],
    morph_definitions: &'scratch mut [DeformationMorphDefinitionValidationProjection],
    logical_morphs: &'scratch mut [DeformationLogicalMorphValidationProjection],
    morph_deltas: &'scratch mut [DeformationMorphDeltaValidationProjection],
    morph_targets: &'scratch mut [DeformationMorphTargetValidationProjection],
    primitives: &'scratch mut [DeformationPrimitiveValidationProjection],
    mesh_instances: &'scratch mut [DeformationMeshInstanceValidationProjection],
    influence_row_offsets: &'scratch mut [u32],
    influence_joints: &'scratch mut [u32],
    influence_source_orders: &'scratch mut [u32],
    palette_maxima: &'scratch mut [Option<u32>],
    uniqueness_marks: &'scratch mut [u64],
}

impl<'scratch> DeformationValidationScratch<'scratch> {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::canonical_scene::store) fn new(
        skins: &'scratch mut [DeformationSkinValidationProjection],
        morph_definitions: &'scratch mut [DeformationMorphDefinitionValidationProjection],
        logical_morphs: &'scratch mut [DeformationLogicalMorphValidationProjection],
        morph_deltas: &'scratch mut [DeformationMorphDeltaValidationProjection],
        morph_targets: &'scratch mut [DeformationMorphTargetValidationProjection],
        primitives: &'scratch mut [DeformationPrimitiveValidationProjection],
        mesh_instances: &'scratch mut [DeformationMeshInstanceValidationProjection],
        influence_row_offsets: &'scratch mut [u32],
        influence_joints: &'scratch mut [u32],
        influence_source_orders: &'scratch mut [u32],
        palette_maxima: &'scratch mut [Option<u32>],
        uniqueness_marks: &'scratch mut [u64],
    ) -> Self {
        Self {
            skins,
            morph_definitions,
            logical_morphs,
            morph_deltas,
            morph_targets,
            primitives,
            mesh_instances,
            influence_row_offsets,
            influence_joints,
            influence_source_orders,
            palette_maxima,
            uniqueness_marks,
        }
    }

    fn validate_capacity(
        &self,
        requirements: DeformationValidationScratchRequirements,
        logical_mesh_count: u32,
    ) -> bool {
        self.skins.len() >= requirements.skins
            && self.morph_definitions.len() >= requirements.morph_definitions
            && self.logical_morphs.len() >= requirements.logical_morphs
            && self.morph_deltas.len() >= requirements.morph_deltas
            && self.morph_targets.len() >= requirements.morph_targets
            && self.primitives.len() >= requirements.primitives
            && self.mesh_instances.len() >= requirements.mesh_instances
            && self.influence_row_offsets.len() >= requirements.influence_row_offsets
            && self.influence_joints.len() >= requirements.influence_joints
            && self.influence_source_orders.len() >= requirements.influence_source_orders
            && usize::try_from(logical_mesh_count)
                .is_ok_and(|count| self.palette_maxima.len() >= count)
    }

    pub(in crate::canonical_scene::store) fn logical_morph_projections(
        &self,
        requirements: DeformationValidationScratchRequirements,
    ) -> Option<&[DeformationLogicalMorphValidationProjection]> {
        self.logical_morphs.get(..requirements.logical_morphs)
    }
}

#[derive(Clone, Copy)]
struct DeformationCanonicalCounts {
    skin_count: u32,
    joint_node_count: u32,
    inverse_bind_scalar_count: u32,
    logical_morph_count: u32,
    primitive_count: u32,
    influence_row_offset_count: u32,
    influence_count: u32,
    morph_target_count: u32,
    morph_delta_count: u32,
    morph_default_count: u32,
    morph_value_count: u32,
    morph_sparse_index_count: u32,
    mesh_instance_count: u32,
    instance_morph_weight_count: u32,
    skin_correction_scalar_count: u32,
    morph_definition_count: u32,
    morph_name_byte_count: u32,
    skin_name_byte_count: u32,
}

#[allow(clippy::too_many_lines)]
fn validate_deformation_directory_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: CanonicalValidationMember<'session, RecordBlock, DeformationDirectorySchema>,
) -> Result<
    Option<(
        DeformationValidatedMembers<'session>,
        DeformationCanonicalCounts,
        u32,
    )>,
    StoreWorkError,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let mut schema_revision = None;
    let mut skin_count = None;
    let mut skins = None;
    let mut joint_node_count = None;
    let mut joint_nodes = None;
    let mut inverse_bind_scalar_count = None;
    let mut inverse_bind_matrices = None;
    let mut logical_morph_count = None;
    let mut logical_morphs = None;
    let mut primitive_count = None;
    let mut primitives = None;
    let mut influence_row_offset_count = None;
    let mut influence_row_offsets = None;
    let mut influence_count = None;
    let mut influence_joints = None;
    let mut influence_weights = None;
    let mut influence_source_orders = None;
    let mut morph_target_count = None;
    let mut morph_targets = None;
    let mut morph_delta_count = None;
    let mut morph_deltas = None;
    let mut morph_default_count = None;
    let mut morph_defaults = None;
    let mut morph_value_count = None;
    let mut morph_values = None;
    let mut morph_sparse_index_count = None;
    let mut morph_sparse_indices = None;
    let mut mesh_instance_count = None;
    let mut mesh_instances = None;
    let mut instance_morph_weight_count = None;
    let mut instance_morph_weights = None;
    let mut skin_correction_scalar_count = None;
    let mut skin_correction_matrices = None;
    let mut morph_definition_count = None;
    let mut morph_definitions = None;
    let mut morph_name_byte_count = None;
    let mut morph_names = None;
    let mut skin_name_byte_count = None;
    let mut skin_names = None;

    let complete = session.visit_canonical_records(
        directory,
        1,
        DEFORMATION_DIRECTORY_FIELD_COUNT,
        |_row_index, row| {
            macro_rules! read_count {
                ($target:ident, $field:expr) => {{
                    $target = row.scalar($field, |value: u32| value)?;
                    let Some(value) = $target else {
                        return Ok(false);
                    };
                    value
                }};
            }
            macro_rules! read_optional_record {
                ($target:ident, $count:expr, $field:expr, $tag:ty) => {
                    if $count == 0 {
                        if row.scalar($field, |value: u8| value)? != Some(0) {
                            return Ok(false);
                        }
                    } else {
                        $target = row.record_ref::<$tag, _>($field, |value| value)?;
                        if $target.is_none() {
                            return Ok(false);
                        }
                    }
                };
            }
            macro_rules! read_optional_sequence {
                ($target:ident, $count:expr, $field:expr, $scalar:ty, $tag:ty) => {
                    if $count == 0 {
                        if row.scalar($field, |value: u8| value)? != Some(0) {
                            return Ok(false);
                        }
                    } else {
                        $target = row.sequence_ref::<$scalar, $tag, _>($field, |value| value)?;
                        if $target.is_none() {
                            return Ok(false);
                        }
                    }
                };
            }
            macro_rules! read_optional_utf8 {
                ($target:ident, $count:expr, $field:expr, $tag:ty) => {
                    if $count == 0 {
                        if row.scalar($field, |value: u8| value)? != Some(0) {
                            return Ok(false);
                        }
                    } else {
                        $target = row.utf8_ref::<$tag, _>($field, |value| value)?;
                        if $target.is_none() {
                            return Ok(false);
                        }
                    }
                };
            }

            schema_revision = row.scalar(
                deformation_directory_field::SCHEMA_REVISION,
                |value: u32| value,
            )?;
            let skin_count_value = read_count!(skin_count, deformation_directory_field::SKIN_COUNT);
            read_optional_record!(
                skins,
                skin_count_value,
                deformation_directory_field::SKINS,
                DeformationSkinTableSchema
            );
            let joint_node_count_value = read_count!(
                joint_node_count,
                deformation_directory_field::JOINT_NODE_COUNT
            );
            read_optional_sequence!(
                joint_nodes,
                joint_node_count_value,
                deformation_directory_field::JOINT_NODES,
                u32,
                DeformationSkinJointNodeSchema
            );
            let inverse_bind_scalar_count_value = read_count!(
                inverse_bind_scalar_count,
                deformation_directory_field::INVERSE_BIND_SCALAR_COUNT
            );
            read_optional_sequence!(
                inverse_bind_matrices,
                inverse_bind_scalar_count_value,
                deformation_directory_field::INVERSE_BIND_MATRICES,
                f32,
                DeformationInverseBindMatrixSchema
            );
            let logical_morph_count_value = read_count!(
                logical_morph_count,
                deformation_directory_field::LOGICAL_MORPH_COUNT
            );
            read_optional_record!(
                logical_morphs,
                logical_morph_count_value,
                deformation_directory_field::LOGICAL_MORPHS,
                DeformationLogicalMorphTableSchema
            );
            let primitive_count_value = read_count!(
                primitive_count,
                deformation_directory_field::PRIMITIVE_DEFORMATION_COUNT
            );
            read_optional_record!(
                primitives,
                primitive_count_value,
                deformation_directory_field::PRIMITIVE_DEFORMATIONS,
                DeformationPrimitiveTableSchema
            );
            let influence_row_offset_count_value = read_count!(
                influence_row_offset_count,
                deformation_directory_field::INFLUENCE_ROW_OFFSET_COUNT
            );
            read_optional_sequence!(
                influence_row_offsets,
                influence_row_offset_count_value,
                deformation_directory_field::INFLUENCE_ROW_OFFSETS,
                u32,
                DeformationInfluenceRowOffsetSchema
            );
            let influence_count_value = read_count!(
                influence_count,
                deformation_directory_field::INFLUENCE_COUNT
            );
            read_optional_sequence!(
                influence_joints,
                influence_count_value,
                deformation_directory_field::INFLUENCE_JOINTS,
                u32,
                DeformationInfluenceJointSchema
            );
            read_optional_sequence!(
                influence_weights,
                influence_count_value,
                deformation_directory_field::INFLUENCE_WEIGHTS,
                f32,
                DeformationInfluenceWeightSchema
            );
            read_optional_sequence!(
                influence_source_orders,
                influence_count_value,
                deformation_directory_field::INFLUENCE_SOURCE_ORDERS,
                u32,
                DeformationInfluenceSourceOrderSchema
            );
            let morph_target_count_value = read_count!(
                morph_target_count,
                deformation_directory_field::MORPH_TARGET_COUNT
            );
            read_optional_record!(
                morph_targets,
                morph_target_count_value,
                deformation_directory_field::MORPH_TARGETS,
                DeformationMorphTargetTableSchema
            );
            let morph_delta_count_value = read_count!(
                morph_delta_count,
                deformation_directory_field::MORPH_DELTA_COUNT
            );
            read_optional_record!(
                morph_deltas,
                morph_delta_count_value,
                deformation_directory_field::MORPH_DELTAS,
                DeformationMorphDeltaTableSchema
            );
            let morph_default_count_value = read_count!(
                morph_default_count,
                deformation_directory_field::MORPH_DEFAULT_COUNT
            );
            read_optional_sequence!(
                morph_defaults,
                morph_default_count_value,
                deformation_directory_field::MORPH_DEFAULTS,
                f32,
                DeformationMorphDefaultWeightSchema
            );
            let morph_value_count_value = read_count!(
                morph_value_count,
                deformation_directory_field::MORPH_VALUE_COUNT
            );
            read_optional_sequence!(
                morph_values,
                morph_value_count_value,
                deformation_directory_field::MORPH_VALUES,
                f32,
                DeformationMorphValueSchema
            );
            let morph_sparse_index_count_value = read_count!(
                morph_sparse_index_count,
                deformation_directory_field::MORPH_SPARSE_INDEX_COUNT
            );
            read_optional_sequence!(
                morph_sparse_indices,
                morph_sparse_index_count_value,
                deformation_directory_field::MORPH_SPARSE_INDICES,
                u32,
                DeformationMorphSparseIndexSchema
            );
            let mesh_instance_count_value = read_count!(
                mesh_instance_count,
                deformation_directory_field::MESH_INSTANCE_COUNT
            );
            read_optional_record!(
                mesh_instances,
                mesh_instance_count_value,
                deformation_directory_field::MESH_INSTANCES,
                DeformationMeshInstanceTableSchema
            );
            let instance_morph_weight_count_value = read_count!(
                instance_morph_weight_count,
                deformation_directory_field::INSTANCE_MORPH_WEIGHT_COUNT
            );
            read_optional_sequence!(
                instance_morph_weights,
                instance_morph_weight_count_value,
                deformation_directory_field::INSTANCE_MORPH_WEIGHTS,
                f32,
                DeformationInstanceMorphWeightSchema
            );
            let skin_correction_scalar_count_value = read_count!(
                skin_correction_scalar_count,
                deformation_directory_field::SKIN_CORRECTION_SCALAR_COUNT
            );
            read_optional_sequence!(
                skin_correction_matrices,
                skin_correction_scalar_count_value,
                deformation_directory_field::SKIN_CORRECTION_MATRICES,
                f32,
                DeformationSkinCorrectionMatrixSchema
            );
            let morph_definition_count_value = read_count!(
                morph_definition_count,
                deformation_directory_field::MORPH_DEFINITION_COUNT
            );
            read_optional_record!(
                morph_definitions,
                morph_definition_count_value,
                deformation_directory_field::MORPH_DEFINITIONS,
                DeformationMorphDefinitionTableSchema
            );
            let morph_name_byte_count_value = read_count!(
                morph_name_byte_count,
                deformation_directory_field::MORPH_NAME_BYTE_COUNT
            );
            read_optional_utf8!(
                morph_names,
                morph_name_byte_count_value,
                deformation_directory_field::MORPH_NAMES,
                DeformationMorphNameSchema
            );
            let skin_name_byte_count_value = read_count!(
                skin_name_byte_count,
                deformation_directory_field::SKIN_NAME_BYTE_COUNT
            );
            read_optional_utf8!(
                skin_names,
                skin_name_byte_count_value,
                deformation_directory_field::SKIN_NAMES,
                DeformationSkinNameSchema
            );
            Ok(schema_revision == Some(DEFORMATION_SCHEMA_REVISION))
        },
    )?;
    if !complete {
        return Ok(None);
    }
    let (
        Some(skin_count),
        Some(joint_node_count),
        Some(inverse_bind_scalar_count),
        Some(logical_morph_count),
        Some(primitive_count),
        Some(influence_row_offset_count),
        Some(influence_count),
        Some(morph_target_count),
        Some(morph_delta_count),
        Some(morph_default_count),
        Some(morph_value_count),
        Some(morph_sparse_index_count),
        Some(mesh_instance_count),
        Some(instance_morph_weight_count),
        Some(skin_correction_scalar_count),
        Some(morph_definition_count),
        Some(morph_name_byte_count),
        Some(skin_name_byte_count),
    ) = (
        skin_count,
        joint_node_count,
        inverse_bind_scalar_count,
        logical_morph_count,
        primitive_count,
        influence_row_offset_count,
        influence_count,
        morph_target_count,
        morph_delta_count,
        morph_default_count,
        morph_value_count,
        morph_sparse_index_count,
        mesh_instance_count,
        instance_morph_weight_count,
        skin_correction_scalar_count,
        morph_definition_count,
        morph_name_byte_count,
        skin_name_byte_count,
    )
    else {
        return Ok(None);
    };
    let payload_block_count = [
        joint_nodes.is_some(),
        inverse_bind_matrices.is_some(),
        skin_correction_matrices.is_some(),
        skin_names.is_some(),
        influence_row_offsets.is_some(),
        influence_joints.is_some(),
        influence_weights.is_some(),
        influence_source_orders.is_some(),
        morph_defaults.is_some(),
        morph_names.is_some(),
        morph_values.is_some(),
        morph_sparse_indices.is_some(),
        instance_morph_weights.is_some(),
    ]
    .into_iter()
    .try_fold(0_u32, |total, present| {
        total.checked_add(u32::from(present))
    });
    let Some(payload_block_count) = payload_block_count else {
        return Ok(None);
    };
    Ok(Some((
        DeformationValidatedMembers {
            directory,
            skins,
            logical_morphs,
            morph_definitions,
            primitives,
            morph_targets,
            morph_deltas,
            mesh_instances,
            joint_nodes,
            inverse_bind_matrices,
            skin_correction_matrices,
            skin_names,
            influence_row_offsets,
            influence_joints,
            influence_weights,
            influence_source_orders,
            morph_defaults,
            morph_names,
            morph_values,
            morph_sparse_indices,
            instance_morph_weights,
        },
        DeformationCanonicalCounts {
            skin_count,
            joint_node_count,
            inverse_bind_scalar_count,
            logical_morph_count,
            primitive_count,
            influence_row_offset_count,
            influence_count,
            morph_target_count,
            morph_delta_count,
            morph_default_count,
            morph_value_count,
            morph_sparse_index_count,
            mesh_instance_count,
            instance_morph_weight_count,
            skin_correction_scalar_count,
            morph_definition_count,
            morph_name_byte_count,
            skin_name_byte_count,
        },
        payload_block_count,
    )))
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn validate_skins_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    members: DeformationValidatedMembers<'session>,
    counts: DeformationCanonicalCounts,
    nodes: CanonicalValidationMember<'session, RecordBlock, NodeSceneNodeTableSchema>,
    node_count: u32,
    scratch: &mut [DeformationSkinValidationProjection],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if counts.skin_count == 0 {
        return Ok(members.skins.is_none());
    }
    let (Some(table), Ok(expected_rows)) = (members.skins, usize::try_from(counts.skin_count))
    else {
        return Ok(false);
    };
    let Some(scratch) = scratch.get_mut(..expected_rows) else {
        return Ok(false);
    };
    let Ok(node_rows) = usize::try_from(node_count) else {
        return Ok(false);
    };
    let mut joint_cursor = 0_u32;
    let mut inverse_cursor = 0_u32;
    let mut correction_cursor = 0_u32;
    let mut name_cursor = 0_u32;
    let complete = session.visit_canonical_records(
        table,
        expected_rows,
        DEFORMATION_SKIN_FIELD_COUNT,
        |row_index, row| {
            let skin = row.scalar(deformation_skin_field::SKIN, |value: u32| value)?;
            let joint_offset =
                row.scalar(deformation_skin_field::JOINT_OFFSET, |value: u32| value)?;
            let joint_count =
                row.scalar(deformation_skin_field::JOINT_COUNT, |value: u32| value)?;
            let Some(joint_count) = joint_count else {
                return Ok(false);
            };
            let joints_match = if joint_count == 0 {
                row.scalar(deformation_skin_field::JOINTS, |value: u8| value)? == Some(0)
            } else {
                row.sequence_ref::<u32, DeformationSkinJointNodeSchema, _>(
                    deformation_skin_field::JOINTS,
                    |value| Some(value) == members.joint_nodes,
                )? == Some(true)
            };
            let inverse_offset = row
                .scalar(deformation_skin_field::INVERSE_BIND_OFFSET, |value: u32| {
                    value
                })?;
            let inverse_count = row
                .scalar(deformation_skin_field::INVERSE_BIND_COUNT, |value: u32| {
                    value
                })?;
            let Some(inverse_count) = inverse_count else {
                return Ok(false);
            };
            let inverse_match = if inverse_count == 0 {
                row.scalar(deformation_skin_field::INVERSE_BINDS, |value: u8| value)? == Some(0)
            } else {
                row.sequence_ref::<f32, DeformationInverseBindMatrixSchema, _>(
                    deformation_skin_field::INVERSE_BINDS,
                    |value| Some(value) == members.inverse_bind_matrices,
                )? == Some(true)
            };
            let skeleton_presence = row.scalar(
                deformation_skin_field::SKELETON_ROOT_PRESENCE,
                |value: u8| value,
            )?;
            let skeleton_root =
                row.scalar(deformation_skin_field::SKELETON_ROOT, |value: u32| value)?;
            let skeleton_present = skeleton_presence == Some(PRESENCE_PRESENT);
            let nodes_match = if joint_count != 0 || skeleton_present {
                row.record_ref::<NodeSceneNodeTableSchema, _>(
                    deformation_skin_field::NODES,
                    |value| value == nodes,
                )? == Some(true)
            } else {
                row.scalar(deformation_skin_field::NODES, |value: u8| value)? == Some(0)
            };
            let input_position_space = row
                .scalar(deformation_skin_field::INPUT_POSITION_SPACE, |value: u8| {
                    value
                })?;
            let inverse_bind_from_space = row.scalar(
                deformation_skin_field::INVERSE_BIND_FROM_SPACE,
                |value: u8| value,
            )?;
            let inverse_bind_to_space = row.scalar(
                deformation_skin_field::INVERSE_BIND_TO_SPACE,
                |value: u8| value,
            )?;
            let current_joint_from_space = row.scalar(
                deformation_skin_field::CURRENT_JOINT_FROM_SPACE,
                |value: u8| value,
            )?;
            let current_joint_to_space = row.scalar(
                deformation_skin_field::CURRENT_JOINT_TO_SPACE,
                |value: u8| value,
            )?;
            let output_position_space = row.scalar(
                deformation_skin_field::OUTPUT_POSITION_SPACE,
                |value: u8| value,
            )?;
            let correction_presence = row
                .scalar(deformation_skin_field::CORRECTION_PRESENCE, |value: u8| {
                    value
                })?;
            let correction_from_space = row.scalar(
                deformation_skin_field::CORRECTION_FROM_SPACE,
                |value: u8| value,
            )?;
            let correction_to_space = row
                .scalar(deformation_skin_field::CORRECTION_TO_SPACE, |value: u8| {
                    value
                })?;
            let correction_offset = row.scalar(
                deformation_skin_field::CORRECTION_MATRIX_OFFSET,
                |value: u32| value,
            )?;
            let correction_count = row.scalar(
                deformation_skin_field::CORRECTION_MATRIX_COUNT,
                |value: u32| value,
            )?;
            let correction_present = correction_presence == Some(PRESENCE_PRESENT);
            let correction_match = if correction_present {
                row.sequence_ref::<f32, DeformationSkinCorrectionMatrixSchema, _>(
                    deformation_skin_field::CORRECTION_MATRICES,
                    |value| Some(value) == members.skin_correction_matrices,
                )? == Some(true)
            } else {
                row.scalar(deformation_skin_field::CORRECTION_MATRICES, |value: u8| {
                    value
                })? == Some(0)
            };
            let name_presence =
                row.scalar(deformation_skin_field::NAME_PRESENCE, |value: u8| value)?;
            let name_offset =
                row.scalar(deformation_skin_field::NAME_OFFSET, |value: u32| value)?;
            let name_count = row.scalar(deformation_skin_field::NAME_COUNT, |value: u32| value)?;
            let name_present = name_presence == Some(PRESENCE_PRESENT);
            let names_match = if name_present && name_count.is_some_and(|count| count != 0) {
                row.utf8_ref::<DeformationSkinNameSchema, _>(
                    deformation_skin_field::NAMES,
                    |value| Some(value) == members.skin_names,
                )? == Some(true)
            } else {
                row.scalar(deformation_skin_field::NAMES, |value: u8| value)? == Some(0)
            };

            let (
                Some(skin),
                Some(joint_offset),
                Some(inverse_offset),
                Some(skeleton_presence),
                Some(skeleton_root),
                Some(correction_presence),
                Some(correction_offset),
                Some(correction_count),
                Some(name_presence),
                Some(name_offset),
                Some(name_count),
            ) = (
                skin,
                joint_offset,
                inverse_offset,
                skeleton_presence,
                skeleton_root,
                correction_presence,
                correction_offset,
                correction_count,
                name_presence,
                name_offset,
                name_count,
            )
            else {
                return Ok(false);
            };
            let Some(expected_skin) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            let Some(inverse_scalar_count) = inverse_count.checked_mul(MATRIX_SCALAR_COUNT) else {
                return Ok(false);
            };
            let skeleton_valid = match skeleton_presence {
                PRESENCE_ABSENT => skeleton_root == 0,
                PRESENCE_PRESENT => is_authored_node_index(skeleton_root, node_rows),
                _ => false,
            };
            let correction_valid = match correction_presence {
                PRESENCE_ABSENT => {
                    correction_from_space == Some(0)
                        && correction_to_space == Some(0)
                        && correction_offset == 0
                        && correction_count == 0
                }
                PRESENCE_PRESENT => {
                    correction_from_space == Some(DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND)
                        && correction_to_space == Some(DEFORMATION_SPACE_MESH_INSTANCE_LOCAL)
                        && correction_offset == correction_cursor
                        && correction_count == MATRIX_SCALAR_COUNT
                }
                _ => false,
            };
            let name_valid = match name_presence {
                PRESENCE_ABSENT => name_offset.checked_add(name_count) == Some(0),
                PRESENCE_PRESENT => name_offset == name_cursor,
                _ => false,
            };
            if skin != expected_skin
                || joint_count != inverse_count
                || joint_offset != joint_cursor
                || inverse_offset != inverse_cursor
                || !joints_match
                || !inverse_match
                || !skeleton_valid
                || !nodes_match
                || input_position_space != Some(DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND)
                || inverse_bind_from_space != Some(DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND)
                || inverse_bind_to_space != Some(DEFORMATION_SPACE_JOINT_LOCAL_BIND)
                || current_joint_from_space != Some(DEFORMATION_SPACE_JOINT_LOCAL)
                || current_joint_to_space != Some(DEFORMATION_SPACE_SCENE)
                || output_position_space != Some(DEFORMATION_SPACE_MESH_INSTANCE_LOCAL)
                || !correction_valid
                || !correction_match
                || !name_valid
                || !names_match
            {
                return Ok(false);
            }
            let Some(slot) = scratch.get_mut(row_index) else {
                return Ok(false);
            };
            *slot = DeformationSkinValidationProjection {
                joint_count,
                name_present,
                name_offset,
                name_count,
            };
            let Some(next_joint_cursor) = joint_cursor.checked_add(joint_count) else {
                return Ok(false);
            };
            let Some(next_inverse_cursor) = inverse_cursor.checked_add(inverse_scalar_count) else {
                return Ok(false);
            };
            let Some(next_correction_cursor) =
                correction_cursor.checked_add(if correction_present {
                    correction_count
                } else {
                    0
                })
            else {
                return Ok(false);
            };
            let Some(next_name_cursor) =
                name_cursor.checked_add(if name_present { name_count } else { 0 })
            else {
                return Ok(false);
            };
            joint_cursor = next_joint_cursor;
            inverse_cursor = next_inverse_cursor;
            correction_cursor = next_correction_cursor;
            name_cursor = next_name_cursor;
            Ok(true)
        },
    )?;
    Ok(complete
        && joint_cursor == counts.joint_node_count
        && inverse_cursor == counts.inverse_bind_scalar_count
        && correction_cursor == counts.skin_correction_scalar_count
        && name_cursor == counts.skin_name_byte_count)
}

#[allow(clippy::too_many_lines)]
fn validate_morph_definitions_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    members: DeformationValidatedMembers<'session>,
    counts: DeformationCanonicalCounts,
    scratch: &mut [DeformationMorphDefinitionValidationProjection],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if counts.morph_definition_count == 0 {
        return Ok(members.morph_definitions.is_none());
    }
    let (Some(table), Ok(expected_rows)) = (
        members.morph_definitions,
        usize::try_from(counts.morph_definition_count),
    ) else {
        return Ok(false);
    };
    let Some(scratch) = scratch.get_mut(..expected_rows) else {
        return Ok(false);
    };
    let mut name_cursor = 0_u32;
    let complete = session.visit_canonical_records(
        table,
        expected_rows,
        DEFORMATION_MORPH_DEFINITION_FIELD_COUNT,
        |row_index, row| {
            let definition = row.scalar(
                deformation_morph_definition_field::MORPH_DEFINITION,
                |value: u32| value,
            )?;
            let logical_mesh = row.scalar(
                deformation_morph_definition_field::LOGICAL_MESH,
                |value: u32| value,
            )?;
            let target_ordinal = row.scalar(
                deformation_morph_definition_field::TARGET_ORDINAL,
                |value: u32| value,
            )?;
            let stable_identity = row.scalar(
                deformation_morph_definition_field::STABLE_IDENTITY,
                |value: u32| value,
            )?;
            let name_presence = row.scalar(
                deformation_morph_definition_field::NAME_PRESENCE,
                |value: u8| value,
            )?;
            let name_offset = row.scalar(
                deformation_morph_definition_field::NAME_OFFSET,
                |value: u32| value,
            )?;
            let name_count = row.scalar(
                deformation_morph_definition_field::NAME_COUNT,
                |value: u32| value,
            )?;
            let name_present = name_presence == Some(PRESENCE_PRESENT);
            let names_match = if name_present && name_count.is_some_and(|count| count != 0) {
                row.utf8_ref::<DeformationMorphNameSchema, _>(
                    deformation_morph_definition_field::NAMES,
                    |value| Some(value) == members.morph_names,
                )? == Some(true)
            } else {
                row.scalar(deformation_morph_definition_field::NAMES, |value: u8| value)? == Some(0)
            };
            let (
                Some(definition),
                Some(logical_mesh),
                Some(target_ordinal),
                Some(stable_identity),
                Some(name_presence),
                Some(name_offset),
                Some(name_count),
            ) = (
                definition,
                logical_mesh,
                target_ordinal,
                stable_identity,
                name_presence,
                name_offset,
                name_count,
            )
            else {
                return Ok(false);
            };
            let Some(expected_definition) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            let name_valid = match name_presence {
                PRESENCE_ABSENT => name_offset.checked_add(name_count) == Some(0),
                PRESENCE_PRESENT => name_offset == name_cursor,
                _ => false,
            };
            if definition != expected_definition
                || logical_mesh >= counts.logical_morph_count
                || !name_valid
                || !names_match
            {
                return Ok(false);
            }
            let Some(slot) = scratch.get_mut(row_index) else {
                return Ok(false);
            };
            *slot = DeformationMorphDefinitionValidationProjection {
                logical_mesh,
                target_ordinal,
                stable_identity,
                name_present,
                name_offset,
                name_count,
            };
            let Some(next_name_cursor) =
                name_cursor.checked_add(if name_present { name_count } else { 0 })
            else {
                return Ok(false);
            };
            name_cursor = next_name_cursor;
            Ok(true)
        },
    )?;
    Ok(complete && name_cursor == counts.morph_name_byte_count)
}

#[allow(clippy::too_many_lines)]
fn validate_logical_morphs_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    members: DeformationValidatedMembers<'session>,
    counts: DeformationCanonicalCounts,
    projections: &mut [DeformationLogicalMorphValidationProjection],
    definitions: &[DeformationMorphDefinitionValidationProjection],
    uniqueness: &mut DeformationUniquenessScratch<'_>,
) -> Result<Option<usize>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if counts.logical_morph_count == 0 {
        return Ok((members.logical_morphs.is_none()
            && counts.morph_definition_count == 0
            && counts.morph_default_count == 0)
            .then_some(0));
    }
    let (Some(table), Ok(expected_rows)) = (
        members.logical_morphs,
        usize::try_from(counts.logical_morph_count),
    ) else {
        return Ok(None);
    };
    let Some(projections) = projections.get_mut(..expected_rows) else {
        return Ok(None);
    };
    let mut definition_cursor = 0_u32;
    let mut default_cursor = 0_u32;
    let complete = session.visit_canonical_records(
        table,
        expected_rows,
        DEFORMATION_LOGICAL_MORPH_FIELD_COUNT,
        |row_index, row| {
            let logical_mesh = row.scalar(
                deformation_logical_morph_field::LOGICAL_MESH,
                |value: u32| value,
            )?;
            let definition_offset = row.scalar(
                deformation_logical_morph_field::DEFINITION_OFFSET,
                |value: u32| value,
            )?;
            let definition_count = row.scalar(
                deformation_logical_morph_field::DEFINITION_COUNT,
                |value: u32| value,
            )?;
            let definitions_match = if definition_count == Some(0) {
                row.scalar(deformation_logical_morph_field::DEFINITIONS, |value: u8| {
                    value
                })? == Some(0)
            } else {
                row.record_ref::<DeformationMorphDefinitionTableSchema, _>(
                    deformation_logical_morph_field::DEFINITIONS,
                    |value| Some(value) == members.morph_definitions,
                )? == Some(true)
            };
            let default_offset = row.scalar(
                deformation_logical_morph_field::DEFAULT_WEIGHT_OFFSET,
                |value: u32| value,
            )?;
            let default_count = row.scalar(
                deformation_logical_morph_field::DEFAULT_WEIGHT_COUNT,
                |value: u32| value,
            )?;
            let defaults_match = if default_count == Some(0) {
                row.scalar(
                    deformation_logical_morph_field::DEFAULT_WEIGHTS,
                    |value: u8| value,
                )? == Some(0)
            } else {
                row.sequence_ref::<f32, DeformationMorphDefaultWeightSchema, _>(
                    deformation_logical_morph_field::DEFAULT_WEIGHTS,
                    |value| Some(value) == members.morph_defaults,
                )? == Some(true)
            };
            let (
                Some(logical_mesh),
                Some(definition_offset),
                Some(definition_count),
                Some(default_offset),
                Some(default_count),
            ) = (
                logical_mesh,
                definition_offset,
                definition_count,
                default_offset,
                default_count,
            )
            else {
                return Ok(false);
            };
            let Some(expected_logical_mesh) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            if logical_mesh != expected_logical_mesh
                || definition_offset != definition_cursor
                || default_offset != default_cursor
                || definition_count != default_count
                || !definitions_match
                || !defaults_match
            {
                return Ok(false);
            }
            let Some(slot) = projections.get_mut(row_index) else {
                return Ok(false);
            };
            *slot = DeformationLogicalMorphValidationProjection {
                definition_offset,
                definition_count,
                default_weight_offset: default_offset,
                default_weight_count: default_count,
            };
            let Some(next_definition_cursor) = definition_cursor.checked_add(definition_count)
            else {
                return Ok(false);
            };
            let Some(next_default_cursor) = default_cursor.checked_add(default_count) else {
                return Ok(false);
            };
            definition_cursor = next_definition_cursor;
            default_cursor = next_default_cursor;
            Ok(true)
        },
    )?;
    if !complete
        || definition_cursor != counts.morph_definition_count
        || default_cursor != counts.morph_default_count
    {
        return Ok(None);
    }

    let Ok(definition_count) = usize::try_from(counts.morph_definition_count) else {
        return Ok(None);
    };
    let Some(definitions) = definitions.get(..definition_count) else {
        return Ok(None);
    };
    let mut maximum_uniqueness_count = 0_usize;
    for (logical_mesh, projection) in projections.iter().copied().enumerate() {
        let Ok(logical_mesh) = u32::try_from(logical_mesh) else {
            return Ok(None);
        };
        let Ok(width) = usize::try_from(projection.definition_count) else {
            return Ok(None);
        };
        maximum_uniqueness_count = maximum_uniqueness_count.max(width);
        let Some(range) = checked_range(
            projection.definition_offset,
            projection.definition_count,
            definitions.len(),
        ) else {
            return Ok(None);
        };
        let Some(mut group) = uniqueness.begin_group(width) else {
            return Ok(None);
        };
        for (target_ordinal, definition) in definitions[range].iter().copied().enumerate() {
            let Ok(target_ordinal) = u32::try_from(target_ordinal) else {
                return Ok(None);
            };
            let Some(valid) = session.run_validation_unit(|| {
                definition.logical_mesh == logical_mesh
                    && definition.target_ordinal == target_ordinal
                    && definition.stable_identity < projection.definition_count
                    && group.mark(definition.stable_identity).is_some()
            })?
            else {
                return Ok(None);
            };
            if !valid {
                return Ok(None);
            }
        }
    }
    Ok(Some(maximum_uniqueness_count))
}

#[allow(clippy::too_many_lines)]
fn validate_morph_deltas_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    members: DeformationValidatedMembers<'session>,
    counts: DeformationCanonicalCounts,
    targets: &[DeformationMorphTargetValidationProjection],
    geometry: &[GeometryPrimitiveValidationProjection],
    projections: &mut [DeformationMorphDeltaValidationProjection],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if counts.morph_delta_count == 0 {
        return Ok(members.morph_deltas.is_none());
    }
    let (Some(table), Ok(expected_rows)) = (
        members.morph_deltas,
        usize::try_from(counts.morph_delta_count),
    ) else {
        return Ok(false);
    };
    let Some(projections) = projections.get_mut(..expected_rows) else {
        return Ok(false);
    };
    let mut value_cursor = 0_u32;
    let mut sparse_cursor = 0_u32;
    let mut previous_target = None;
    let mut previous_semantic = None;
    let complete = session.visit_canonical_records(
        table,
        expected_rows,
        DEFORMATION_MORPH_DELTA_FIELD_COUNT,
        |row_index, row| {
            let morph_delta = row
                .scalar(deformation_morph_delta_field::MORPH_DELTA, |value: u32| {
                    value
                })?;
            let morph_target = row
                .scalar(deformation_morph_delta_field::MORPH_TARGET, |value: u32| {
                    value
                })?;
            let semantic =
                row.scalar(deformation_morph_delta_field::SEMANTIC, |value: u8| value)?;
            let payload_kind = row
                .scalar(deformation_morph_delta_field::PAYLOAD_KIND, |value: u8| {
                    value
                })?;
            let logical_count = row.scalar(
                deformation_morph_delta_field::LOGICAL_COUNT,
                |value: u32| value,
            )?;
            let explicit_count = row.scalar(
                deformation_morph_delta_field::EXPLICIT_COUNT,
                |value: u32| value,
            )?;
            let value_offset = row
                .scalar(deformation_morph_delta_field::VALUE_OFFSET, |value: u32| {
                    value
                })?;
            let value_count = row
                .scalar(deformation_morph_delta_field::VALUE_COUNT, |value: u32| {
                    value
                })?;
            let values_match = if value_count == Some(0) {
                row.scalar(deformation_morph_delta_field::VALUES, |value: u8| value)? == Some(0)
            } else {
                row.sequence_ref::<f32, DeformationMorphValueSchema, _>(
                    deformation_morph_delta_field::VALUES,
                    |value| Some(value) == members.morph_values,
                )? == Some(true)
            };
            let sparse_offset = row.scalar(
                deformation_morph_delta_field::SPARSE_INDEX_OFFSET,
                |value: u32| value,
            )?;
            let sparse_count = row.scalar(
                deformation_morph_delta_field::SPARSE_INDEX_COUNT,
                |value: u32| value,
            )?;
            let sparse_match = if sparse_count == Some(0) {
                row.scalar(
                    deformation_morph_delta_field::SPARSE_INDICES,
                    |value: u8| value,
                )? == Some(0)
            } else {
                row.sequence_ref::<u32, DeformationMorphSparseIndexSchema, _>(
                    deformation_morph_delta_field::SPARSE_INDICES,
                    |value| Some(value) == members.morph_sparse_indices,
                )? == Some(true)
            };
            let (
                Some(morph_delta),
                Some(morph_target),
                Some(semantic),
                Some(payload_kind),
                Some(logical_count),
                Some(explicit_count),
                Some(value_offset),
                Some(value_count),
                Some(sparse_offset),
                Some(sparse_count),
            ) = (
                morph_delta,
                morph_target,
                semantic,
                payload_kind,
                logical_count,
                explicit_count,
                value_offset,
                value_count,
                sparse_offset,
                sparse_count,
            )
            else {
                return Ok(false);
            };
            let Some(expected_delta) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            let target_valid = usize::try_from(morph_target)
                .ok()
                .and_then(|index| targets.get(index))
                .is_some_and(|target| {
                    let delta_end = target.delta_offset.checked_add(target.delta_count);
                    let logical_matches = usize::try_from(target.primitive)
                        .ok()
                        .and_then(|index| geometry.get(index))
                        .is_some_and(|primitive| primitive.vertex_count == logical_count);
                    target.delta_offset <= expected_delta
                        && delta_end.is_some_and(|end| expected_delta < end)
                        && logical_matches
                });
            let semantic_valid = matches!(
                semantic,
                DEFORMATION_MORPH_POSITION | DEFORMATION_MORPH_NORMAL | DEFORMATION_MORPH_TANGENT
            ) && if previous_target == Some(morph_target) {
                previous_semantic.is_none_or(|previous| semantic > previous)
            } else {
                true
            };
            let payload_valid = match payload_kind {
                DEFORMATION_MORPH_DENSE => explicit_count == logical_count && sparse_count == 0,
                DEFORMATION_MORPH_SPARSE => sparse_count == explicit_count,
                _ => false,
            };
            if morph_delta != expected_delta
                || morph_target >= counts.morph_target_count
                || !target_valid
                || !semantic_valid
                || !payload_valid
                || explicit_count.checked_mul(MORPH_VECTOR_WIDTH) != Some(value_count)
                || value_offset != value_cursor
                || sparse_offset != sparse_cursor
                || !values_match
                || !sparse_match
            {
                return Ok(false);
            }
            let Some(slot) = projections.get_mut(row_index) else {
                return Ok(false);
            };
            *slot = DeformationMorphDeltaValidationProjection {
                logical_count,
                sparse_index_offset: sparse_offset,
                sparse_index_count: sparse_count,
            };
            let Some(next_value_cursor) = value_cursor.checked_add(value_count) else {
                return Ok(false);
            };
            let Some(next_sparse_cursor) = sparse_cursor.checked_add(sparse_count) else {
                return Ok(false);
            };
            value_cursor = next_value_cursor;
            sparse_cursor = next_sparse_cursor;
            previous_target = Some(morph_target);
            previous_semantic = Some(semantic);
            Ok(true)
        },
    )?;
    Ok(complete
        && value_cursor == counts.morph_value_count
        && sparse_cursor == counts.morph_sparse_index_count)
}

#[allow(clippy::too_many_lines)]
fn validate_morph_targets_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    members: DeformationValidatedMembers<'session>,
    counts: DeformationCanonicalCounts,
    primitives: &[DeformationPrimitiveValidationProjection],
    projections: &mut [DeformationMorphTargetValidationProjection],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if counts.morph_target_count == 0 {
        return Ok(members.morph_targets.is_none());
    }
    let (Some(table), Ok(expected_rows)) = (
        members.morph_targets,
        usize::try_from(counts.morph_target_count),
    ) else {
        return Ok(false);
    };
    let Some(projections) = projections.get_mut(..expected_rows) else {
        return Ok(false);
    };
    let mut delta_cursor = 0_u32;
    let complete = session.visit_canonical_records(
        table,
        expected_rows,
        DEFORMATION_MORPH_TARGET_FIELD_COUNT,
        |row_index, row| {
            let morph_target = row.scalar(
                deformation_morph_target_field::MORPH_TARGET,
                |value: u32| value,
            )?;
            let primitive = row
                .scalar(deformation_morph_target_field::PRIMITIVE, |value: u32| {
                    value
                })?;
            let target_ordinal = row.scalar(
                deformation_morph_target_field::TARGET_ORDINAL,
                |value: u32| value,
            )?;
            let delta_offset = row.scalar(
                deformation_morph_target_field::DELTA_OFFSET,
                |value: u32| value,
            )?;
            let delta_count = row
                .scalar(deformation_morph_target_field::DELTA_COUNT, |value: u32| {
                    value
                })?;
            let deltas_match = if delta_count == Some(0) {
                row.scalar(deformation_morph_target_field::DELTAS, |value: u8| value)? == Some(0)
            } else {
                row.record_ref::<DeformationMorphDeltaTableSchema, _>(
                    deformation_morph_target_field::DELTAS,
                    |value| Some(value) == members.morph_deltas,
                )? == Some(true)
            };
            let (
                Some(morph_target),
                Some(primitive),
                Some(target_ordinal),
                Some(delta_offset),
                Some(delta_count),
            ) = (
                morph_target,
                primitive,
                target_ordinal,
                delta_offset,
                delta_count,
            )
            else {
                return Ok(false);
            };
            let Some(expected_target) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            let primitive_valid = usize::try_from(primitive)
                .ok()
                .and_then(|index| primitives.get(index))
                .is_some_and(|projection| {
                    projection.morph_target_offset <= expected_target
                        && projection
                            .morph_target_offset
                            .checked_add(projection.morph_target_count)
                            .is_some_and(|end| expected_target < end)
                        && expected_target.checked_sub(projection.morph_target_offset)
                            == Some(target_ordinal)
                });
            if morph_target != expected_target
                || primitive >= counts.primitive_count
                || !primitive_valid
                || delta_offset != delta_cursor
                || !deltas_match
            {
                return Ok(false);
            }
            let Some(slot) = projections.get_mut(row_index) else {
                return Ok(false);
            };
            *slot = DeformationMorphTargetValidationProjection {
                primitive,
                delta_offset,
                delta_count,
            };
            let Some(next_delta_cursor) = delta_cursor.checked_add(delta_count) else {
                return Ok(false);
            };
            delta_cursor = next_delta_cursor;
            Ok(true)
        },
    )?;
    Ok(complete && delta_cursor == counts.morph_delta_count)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn validate_primitives_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    members: DeformationValidatedMembers<'session>,
    counts: DeformationCanonicalCounts,
    geometry: &[GeometryPrimitiveValidationProjection],
    projections: &mut [DeformationPrimitiveValidationProjection],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if counts.primitive_count == 0 {
        return Ok(members.primitives.is_none()
            && counts.influence_row_offset_count == 0
            && counts.influence_count == 0
            && counts.morph_target_count == 0);
    }
    let (Some(table), Ok(expected_rows)) =
        (members.primitives, usize::try_from(counts.primitive_count))
    else {
        return Ok(false);
    };
    let (Some(geometry), Some(projections)) = (
        geometry.get(..expected_rows),
        projections.get_mut(..expected_rows),
    ) else {
        return Ok(false);
    };
    let mut row_offset_cursor = 0_u32;
    let mut influence_cursor = 0_u32;
    let mut target_cursor = 0_u32;
    let complete = session.visit_canonical_records(
        table,
        expected_rows,
        DEFORMATION_PRIMITIVE_FIELD_COUNT,
        |row_index, row| {
            let primitive =
                row.scalar(deformation_primitive_field::PRIMITIVE, |value: u32| value)?;
            let influence_presence = row.scalar(
                deformation_primitive_field::INFLUENCE_PRESENCE,
                |value: u8| value,
            )?;
            let row_offset_offset = row.scalar(
                deformation_primitive_field::ROW_OFFSET_OFFSET,
                |value: u32| value,
            )?;
            let row_offset_count = row.scalar(
                deformation_primitive_field::ROW_OFFSET_COUNT,
                |value: u32| value,
            )?;
            let influences_present = influence_presence == Some(PRESENCE_PRESENT);
            let row_offsets_match = if influences_present {
                row.sequence_ref::<u32, DeformationInfluenceRowOffsetSchema, _>(
                    deformation_primitive_field::ROW_OFFSETS,
                    |value| Some(value) == members.influence_row_offsets,
                )? == Some(true)
            } else {
                row.scalar(deformation_primitive_field::ROW_OFFSETS, |value: u8| value)? == Some(0)
            };
            let influence_offset = row.scalar(
                deformation_primitive_field::INFLUENCE_OFFSET,
                |value: u32| value,
            )?;
            let influence_count = row.scalar(
                deformation_primitive_field::INFLUENCE_COUNT,
                |value: u32| value,
            )?;
            let influence_payload_present = influence_count.is_some_and(|count| count != 0);
            let joints_match = if influence_payload_present {
                row.sequence_ref::<u32, DeformationInfluenceJointSchema, _>(
                    deformation_primitive_field::INFLUENCE_JOINTS,
                    |value| Some(value) == members.influence_joints,
                )? == Some(true)
            } else {
                row.scalar(
                    deformation_primitive_field::INFLUENCE_JOINTS,
                    |value: u8| value,
                )? == Some(0)
            };
            let weights_match = if influence_payload_present {
                row.sequence_ref::<f32, DeformationInfluenceWeightSchema, _>(
                    deformation_primitive_field::INFLUENCE_WEIGHTS,
                    |value| Some(value) == members.influence_weights,
                )? == Some(true)
            } else {
                row.scalar(
                    deformation_primitive_field::INFLUENCE_WEIGHTS,
                    |value: u8| value,
                )? == Some(0)
            };
            let source_orders_match = if influence_payload_present {
                row.sequence_ref::<u32, DeformationInfluenceSourceOrderSchema, _>(
                    deformation_primitive_field::INFLUENCE_SOURCE_ORDERS,
                    |value| Some(value) == members.influence_source_orders,
                )? == Some(true)
            } else {
                row.scalar(
                    deformation_primitive_field::INFLUENCE_SOURCE_ORDERS,
                    |value: u8| value,
                )? == Some(0)
            };
            let target_offset = row.scalar(
                deformation_primitive_field::MORPH_TARGET_OFFSET,
                |value: u32| value,
            )?;
            let target_count = row.scalar(
                deformation_primitive_field::MORPH_TARGET_COUNT,
                |value: u32| value,
            )?;
            let targets_match = if target_count == Some(0) {
                row.scalar(deformation_primitive_field::MORPH_TARGETS, |value: u8| {
                    value
                })? == Some(0)
            } else {
                row.record_ref::<DeformationMorphTargetTableSchema, _>(
                    deformation_primitive_field::MORPH_TARGETS,
                    |value| Some(value) == members.morph_targets,
                )? == Some(true)
            };
            let (
                Some(primitive),
                Some(influence_presence),
                Some(row_offset_offset),
                Some(row_offset_count),
                Some(influence_offset),
                Some(influence_count),
                Some(target_offset),
                Some(target_count),
            ) = (
                primitive,
                influence_presence,
                row_offset_offset,
                row_offset_count,
                influence_offset,
                influence_count,
                target_offset,
                target_count,
            )
            else {
                return Ok(false);
            };
            let Some(expected_primitive) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            let influence_valid = match influence_presence {
                PRESENCE_ABSENT => {
                    row_offset_offset == 0
                        && row_offset_count == 0
                        && influence_offset == 0
                        && influence_count == 0
                }
                PRESENCE_PRESENT => {
                    row_offset_offset == row_offset_cursor
                        && influence_offset == influence_cursor
                        && geometry
                            .get(row_index)
                            .and_then(|shape| shape.vertex_count.checked_add(1))
                            == Some(row_offset_count)
                }
                _ => false,
            };
            if primitive != expected_primitive
                || !influence_valid
                || !row_offsets_match
                || !joints_match
                || !weights_match
                || !source_orders_match
                || target_offset != target_cursor
                || !targets_match
            {
                return Ok(false);
            }
            let Some(slot) = projections.get_mut(row_index) else {
                return Ok(false);
            };
            *slot = DeformationPrimitiveValidationProjection {
                influences_present,
                row_offset_offset,
                row_offset_count,
                influence_offset,
                influence_count,
                morph_target_offset: target_offset,
                morph_target_count: target_count,
            };
            if influences_present {
                let Some(next_row_offset_cursor) = row_offset_cursor.checked_add(row_offset_count)
                else {
                    return Ok(false);
                };
                let Some(next_influence_cursor) = influence_cursor.checked_add(influence_count)
                else {
                    return Ok(false);
                };
                row_offset_cursor = next_row_offset_cursor;
                influence_cursor = next_influence_cursor;
            }
            let Some(next_target_cursor) = target_cursor.checked_add(target_count) else {
                return Ok(false);
            };
            target_cursor = next_target_cursor;
            Ok(true)
        },
    )?;
    Ok(complete
        && row_offset_cursor == counts.influence_row_offset_count
        && influence_cursor == counts.influence_count
        && target_cursor == counts.morph_target_count)
}

#[allow(clippy::too_many_lines)]
fn validate_mesh_instances_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    members: DeformationValidatedMembers<'session>,
    counts: DeformationCanonicalCounts,
    geometry: &[GeometryMeshInstanceValidationProjection],
    logical_morphs: &[DeformationLogicalMorphValidationProjection],
    projections: &mut [DeformationMeshInstanceValidationProjection],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if counts.mesh_instance_count == 0 {
        return Ok(members.mesh_instances.is_none() && counts.instance_morph_weight_count == 0);
    }
    let (Some(table), Ok(expected_rows)) = (
        members.mesh_instances,
        usize::try_from(counts.mesh_instance_count),
    ) else {
        return Ok(false);
    };
    let (Some(geometry), Some(projections)) = (
        geometry.get(..expected_rows),
        projections.get_mut(..expected_rows),
    ) else {
        return Ok(false);
    };
    let mut weight_cursor = 0_u32;
    let complete = session.visit_canonical_records(
        table,
        expected_rows,
        DEFORMATION_MESH_INSTANCE_FIELD_COUNT,
        |row_index, row| {
            let mesh_instance = row.scalar(
                deformation_mesh_instance_field::MESH_INSTANCE,
                |value: u32| value,
            )?;
            let skin_presence = row.scalar(
                deformation_mesh_instance_field::SKIN_PRESENCE,
                |value: u8| value,
            )?;
            let skin = row.scalar(deformation_mesh_instance_field::SKIN, |value: u32| value)?;
            let skin_present = skin_presence == Some(PRESENCE_PRESENT);
            let skins_match = if skin_present {
                row.record_ref::<DeformationSkinTableSchema, _>(
                    deformation_mesh_instance_field::SKINS,
                    |value| Some(value) == members.skins,
                )? == Some(true)
            } else {
                row.scalar(deformation_mesh_instance_field::SKINS, |value: u8| value)? == Some(0)
            };
            let weights_presence = row.scalar(
                deformation_mesh_instance_field::MORPH_WEIGHTS_PRESENCE,
                |value: u8| value,
            )?;
            let weight_offset = row.scalar(
                deformation_mesh_instance_field::MORPH_WEIGHT_OFFSET,
                |value: u32| value,
            )?;
            let weight_count = row.scalar(
                deformation_mesh_instance_field::MORPH_WEIGHT_COUNT,
                |value: u32| value,
            )?;
            let weights_present = weights_presence == Some(PRESENCE_PRESENT);
            let weights_match = if weights_present && weight_count.is_some_and(|count| count != 0) {
                row.sequence_ref::<f32, DeformationInstanceMorphWeightSchema, _>(
                    deformation_mesh_instance_field::MORPH_WEIGHTS,
                    |value| Some(value) == members.instance_morph_weights,
                )? == Some(true)
            } else {
                row.scalar(
                    deformation_mesh_instance_field::MORPH_WEIGHTS,
                    |value: u8| value,
                )? == Some(0)
            };
            let (
                Some(mesh_instance),
                Some(skin_presence),
                Some(skin),
                Some(weights_presence),
                Some(weight_offset),
                Some(weight_count),
            ) = (
                mesh_instance,
                skin_presence,
                skin,
                weights_presence,
                weight_offset,
                weight_count,
            )
            else {
                return Ok(false);
            };
            let Some(expected_instance) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            let skin_valid = match skin_presence {
                PRESENCE_ABSENT => skin == 0,
                PRESENCE_PRESENT => skin < counts.skin_count,
                _ => false,
            };
            let Some(logical_mesh) = geometry.get(row_index).map(|shape| shape.logical_mesh) else {
                return Ok(false);
            };
            let Some(target_count) = usize::try_from(logical_mesh)
                .ok()
                .and_then(|index| logical_morphs.get(index))
                .map(|projection| projection.definition_count)
            else {
                return Ok(false);
            };
            let weights_valid = match weights_presence {
                PRESENCE_ABSENT => weight_offset.checked_add(weight_count) == Some(0),
                PRESENCE_PRESENT => weight_offset == weight_cursor && weight_count == target_count,
                _ => false,
            };
            if mesh_instance != expected_instance
                || !skin_valid
                || !skins_match
                || !weights_valid
                || !weights_match
            {
                return Ok(false);
            }
            let Some(slot) = projections.get_mut(row_index) else {
                return Ok(false);
            };
            *slot = DeformationMeshInstanceValidationProjection { skin_present, skin };
            let Some(next_weight_cursor) =
                weight_cursor.checked_add(if weights_present { weight_count } else { 0 })
            else {
                return Ok(false);
            };
            weight_cursor = next_weight_cursor;
            Ok(true)
        },
    )?;
    Ok(complete && weight_cursor == counts.instance_morph_weight_count)
}

trait DeformationNameProjection {
    fn name_present(self) -> bool;
    fn name_offset(self) -> u32;
    fn name_count(self) -> u32;
}

impl DeformationNameProjection for DeformationSkinValidationProjection {
    fn name_present(self) -> bool {
        self.name_present
    }

    fn name_offset(self) -> u32 {
        self.name_offset
    }

    fn name_count(self) -> u32 {
        self.name_count
    }
}

impl DeformationNameProjection for DeformationMorphDefinitionValidationProjection {
    fn name_present(self) -> bool {
        self.name_present
    }

    fn name_offset(self) -> u32 {
        self.name_offset
    }

    fn name_count(self) -> u32 {
        self.name_count
    }
}

fn validate_names_metered<'session, 'storage, 'control, 'operation, O, PP, U, Tag, P>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<CanonicalValidationMember<'session, Utf8Block, Tag>>,
    byte_count: u32,
    projections: &[P],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Tag: Copy + Send + Sync + 'static,
    P: Copy + DeformationNameProjection,
{
    let Ok(expected_len) = usize::try_from(byte_count) else {
        return Ok(false);
    };
    if expected_len == 0 {
        return Ok(member.is_none()
            && projections.iter().copied().all(|projection| {
                !projection.name_present()
                    || (projection.name_offset() == 0
                        && projection.name_count() == 0
                        && validate_entity_name_syntax("").is_some())
            }));
    }
    let Some(member) = member else {
        return Ok(false);
    };
    let mut name_buffer = [0_u8; MAX_ENTITY_NAME_BYTES];
    let mut projection_index = 0_usize;
    let mut valid = true;
    let complete = session.visit_canonical_utf8_bytes(member, expected_len, |index, value| {
        if !valid {
            return false;
        }
        while let Some(projection) = projections.get(projection_index).copied() {
            if !projection.name_present() {
                projection_index += 1;
                continue;
            }
            let Ok(offset) = usize::try_from(projection.name_offset()) else {
                valid = false;
                return false;
            };
            let Ok(count) = usize::try_from(projection.name_count()) else {
                valid = false;
                return false;
            };
            if count > MAX_ENTITY_NAME_BYTES {
                valid = false;
                return false;
            }
            if count == 0 {
                valid &= offset == index && validate_entity_name_syntax("").is_some();
                projection_index += 1;
                continue;
            }
            if index < offset || index >= offset.saturating_add(count) {
                valid = false;
                return false;
            }
            let local = index - offset;
            let Some(slot) = name_buffer.get_mut(local) else {
                valid = false;
                return false;
            };
            *slot = value;
            if local + 1 == count {
                valid = std::str::from_utf8(&name_buffer[..count])
                    .ok()
                    .and_then(validate_entity_name_syntax)
                    .is_some();
                projection_index += 1;
            }
            return valid;
        }
        valid = false;
        false
    })?;
    while valid {
        let Some(projection) = projections.get(projection_index).copied() else {
            break;
        };
        if projection.name_present() {
            valid = projection.name_offset() == byte_count
                && projection.name_count() == 0
                && validate_entity_name_syntax("").is_some();
        }
        projection_index += 1;
    }
    Ok(complete && valid && projection_index == projections.len())
}

fn validate_matrix_sequence_metered<'session, 'storage, 'control, 'operation, O, PP, U, Tag>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<CanonicalValidationMember<'session, ScalarSequence<f32>, Tag>>,
    scalar_count: u32,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Tag: Copy + Send + Sync + 'static,
{
    let Ok(expected_len) = usize::try_from(scalar_count) else {
        return Ok(false);
    };
    if expected_len == 0 {
        return Ok(member.is_none());
    }
    if !scalar_count.is_multiple_of(MATRIX_SCALAR_COUNT) {
        return Ok(false);
    }
    let Some(member) = member else {
        return Ok(false);
    };
    session.visit_canonical_sequence(member, expected_len, |index, value| {
        value.is_finite()
            && match index % 16 {
                3 | 7 | 11 => value == 0.0,
                15 => value == 1.0,
                _ => true,
            }
    })
}

fn validate_influence_row_offsets_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<
        CanonicalValidationMember<
            'session,
            ScalarSequence<u32>,
            DeformationInfluenceRowOffsetSchema,
        >,
    >,
    count: u32,
    primitives: &[DeformationPrimitiveValidationProjection],
    output: &mut [u32],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Ok(expected_len) = usize::try_from(count) else {
        return Ok(false);
    };
    if expected_len == 0 {
        return Ok(member.is_none());
    }
    let (Some(member), Some(output)) = (member, output.get_mut(..expected_len)) else {
        return Ok(false);
    };
    let mut primitive_index = 0_usize;
    let mut previous = 0_u32;
    session.visit_canonical_sequence(member, expected_len, |index, value| {
        while primitives.get(primitive_index).is_some_and(|primitive| {
            !primitive.influences_present
                || usize::try_from(
                    primitive
                        .row_offset_offset
                        .saturating_add(primitive.row_offset_count),
                )
                .is_ok_and(|end| index >= end)
        }) {
            primitive_index += 1;
            previous = 0;
        }
        let Some(primitive) = primitives.get(primitive_index).copied() else {
            return false;
        };
        let Ok(start) = usize::try_from(primitive.row_offset_offset) else {
            return false;
        };
        let Ok(segment_len) = usize::try_from(primitive.row_offset_count) else {
            return false;
        };
        let local = index.saturating_sub(start);
        let valid = index >= start
            && local < segment_len
            && value <= primitive.influence_count
            && (local == 0 || value >= previous)
            && (local != 0 || value == 0)
            && (local + 1 != segment_len || value == primitive.influence_count);
        output[index] = value;
        previous = value;
        valid
    })
}

fn validate_sparse_indices_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<
        CanonicalValidationMember<'session, ScalarSequence<u32>, DeformationMorphSparseIndexSchema>,
    >,
    count: u32,
    deltas: &[DeformationMorphDeltaValidationProjection],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Ok(expected_len) = usize::try_from(count) else {
        return Ok(false);
    };
    if expected_len == 0 {
        return Ok(member.is_none());
    }
    let Some(member) = member else {
        return Ok(false);
    };
    let mut delta_index = 0_usize;
    let mut previous = None;
    session.visit_canonical_sequence(member, expected_len, |index, value| {
        while deltas.get(delta_index).is_some_and(|delta| {
            usize::try_from(
                delta
                    .sparse_index_offset
                    .saturating_add(delta.sparse_index_count),
            )
            .is_ok_and(|end| index >= end)
        }) {
            delta_index += 1;
            previous = None;
        }
        let Some(delta) = deltas.get(delta_index).copied() else {
            return false;
        };
        let Ok(start) = usize::try_from(delta.sparse_index_offset) else {
            return false;
        };
        let valid = index >= start
            && value < delta.logical_count
            && previous.is_none_or(|previous| value > previous);
        previous = Some(value);
        valid
    })
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn validate_deformation_payloads_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    members: DeformationValidatedMembers<'session>,
    counts: DeformationCanonicalCounts,
    node_count: u32,
    skins: &[DeformationSkinValidationProjection],
    morph_definitions: &[DeformationMorphDefinitionValidationProjection],
    logical_morphs: &[DeformationLogicalMorphValidationProjection],
    morph_deltas: &[DeformationMorphDeltaValidationProjection],
    primitives: &[DeformationPrimitiveValidationProjection],
    influence_row_offsets: &mut [u32],
    influence_joints: &mut [u32],
    influence_source_orders: &mut [u32],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Ok(node_rows) = usize::try_from(node_count) else {
        return Ok(false);
    };
    let joint_nodes_valid = match (
        members.joint_nodes,
        usize::try_from(counts.joint_node_count),
    ) {
        (None, Ok(0)) => true,
        (Some(member), Ok(expected_len)) if expected_len != 0 => {
            session.visit_canonical_sequence(member, expected_len, |_index, value| {
                is_authored_node_index(value, node_rows)
            })?
        }
        _ => false,
    };
    if !joint_nodes_valid
        || !validate_matrix_sequence_metered(
            session,
            members.inverse_bind_matrices,
            counts.inverse_bind_scalar_count,
        )?
        || !validate_matrix_sequence_metered(
            session,
            members.skin_correction_matrices,
            counts.skin_correction_scalar_count,
        )?
        || !validate_names_metered(
            session,
            members.skin_names,
            counts.skin_name_byte_count,
            skins,
        )?
        || !validate_names_metered(
            session,
            members.morph_names,
            counts.morph_name_byte_count,
            morph_definitions,
        )?
    {
        return Ok(false);
    }

    let morph_defaults_valid = match (
        members.morph_defaults,
        usize::try_from(counts.morph_default_count),
    ) {
        (None, Ok(0)) => true,
        (Some(member), Ok(expected_len)) if expected_len != 0 => {
            session
                .visit_canonical_sequence(member, expected_len, |_index, value| value.is_finite())?
        }
        _ => false,
    };
    let morph_values_valid = match (
        members.morph_values,
        usize::try_from(counts.morph_value_count),
    ) {
        (None, Ok(0)) => true,
        (Some(member), Ok(expected_len)) if expected_len != 0 => {
            session
                .visit_canonical_sequence(member, expected_len, |_index, value| value.is_finite())?
        }
        _ => false,
    };
    if !morph_defaults_valid
        || !morph_values_valid
        || !validate_sparse_indices_metered(
            session,
            members.morph_sparse_indices,
            counts.morph_sparse_index_count,
            morph_deltas,
        )?
        || !validate_influence_row_offsets_metered(
            session,
            members.influence_row_offsets,
            counts.influence_row_offset_count,
            primitives,
            influence_row_offsets,
        )?
    {
        return Ok(false);
    }

    let Ok(influence_len) = usize::try_from(counts.influence_count) else {
        return Ok(false);
    };
    let (Some(influence_joints_output), Some(source_orders_output)) = (
        influence_joints.get_mut(..influence_len),
        influence_source_orders.get_mut(..influence_len),
    ) else {
        return Ok(false);
    };
    let influence_joints_valid = match members.influence_joints {
        None => influence_len == 0,
        Some(member) if influence_len != 0 => {
            session.visit_canonical_sequence(member, influence_len, |index, value| {
                influence_joints_output[index] = value;
                true
            })?
        }
        Some(_) => false,
    };
    let influence_weights_valid = match members.influence_weights {
        None => influence_len == 0,
        Some(member) if influence_len != 0 => {
            session.visit_canonical_sequence(member, influence_len, |_index, value| {
                value.is_finite() && value >= 0.0
            })?
        }
        Some(_) => false,
    };
    let source_orders_valid = match members.influence_source_orders {
        None => influence_len == 0,
        Some(member) if influence_len != 0 => {
            session.visit_canonical_sequence(member, influence_len, |index, value| {
                source_orders_output[index] = value;
                true
            })?
        }
        Some(_) => false,
    };
    let instance_weights_valid = match (
        members.instance_morph_weights,
        usize::try_from(counts.instance_morph_weight_count),
    ) {
        (None, Ok(0)) => true,
        (Some(member), Ok(expected_len)) if expected_len != 0 => {
            session
                .visit_canonical_sequence(member, expected_len, |_index, value| value.is_finite())?
        }
        _ => false,
    };

    Ok(influence_joints_valid
        && influence_weights_valid
        && source_orders_valid
        && instance_weights_valid
        && logical_morphs.iter().try_fold(0_u32, |cursor, projection| {
            if projection.default_weight_offset == cursor {
                cursor.checked_add(projection.default_weight_count)
            } else {
                None
            }
        }) == Some(counts.morph_default_count))
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn validate_deformation_algorithms_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    counts: DeformationCanonicalCounts,
    geometry: DeformationGeometryValidationProjection<'_>,
    skins: &[DeformationSkinValidationProjection],
    logical_morphs: &[DeformationLogicalMorphValidationProjection],
    morph_targets: &[DeformationMorphTargetValidationProjection],
    primitives: &[DeformationPrimitiveValidationProjection],
    mesh_instances: &[DeformationMeshInstanceValidationProjection],
    influence_row_offsets: &[u32],
    influence_joints: &[u32],
    influence_source_orders: &[u32],
    palette_maxima: &mut [Option<u32>],
    uniqueness: &mut DeformationUniquenessScratch<'_>,
) -> Result<Option<usize>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let (Ok(primitive_count), Ok(target_count), Ok(influence_count), Ok(instance_count)) = (
        usize::try_from(counts.primitive_count),
        usize::try_from(counts.morph_target_count),
        usize::try_from(counts.influence_count),
        usize::try_from(counts.mesh_instance_count),
    ) else {
        return Ok(None);
    };
    let (
        Some(geometry_primitives),
        Some(primitives),
        Some(targets),
        Some(instances),
        Some(influence_joints),
        Some(source_orders),
    ) = (
        geometry.primitives.get(..primitive_count),
        primitives.get(..primitive_count),
        morph_targets.get(..target_count),
        mesh_instances.get(..instance_count),
        influence_joints.get(..influence_count),
        influence_source_orders.get(..influence_count),
    )
    else {
        return Ok(None);
    };

    let mut target_cursor = 0_u32;
    for (primitive_index, primitive) in primitives.iter().copied().enumerate() {
        let Some(valid) = session.run_validation_unit(|| {
            primitive.morph_target_offset == target_cursor
                && primitive
                    .morph_target_offset
                    .checked_add(primitive.morph_target_count)
                    .is_some_and(|end| end <= counts.morph_target_count)
                && geometry_primitives.get(primitive_index).is_some()
        })?
        else {
            return Ok(None);
        };
        if !valid {
            return Ok(None);
        }
        target_cursor = match target_cursor.checked_add(primitive.morph_target_count) {
            Some(cursor) => cursor,
            None => return Ok(None),
        };
    }
    if target_cursor != counts.morph_target_count {
        return Ok(None);
    }

    let mut delta_cursor = 0_u32;
    for (target_index, target) in targets.iter().copied().enumerate() {
        let Ok(target_index) = u32::try_from(target_index) else {
            return Ok(None);
        };
        let Some(valid) = session.run_validation_unit(|| {
            target.delta_offset == delta_cursor
                && target
                    .delta_offset
                    .checked_add(target.delta_count)
                    .is_some_and(|end| end <= counts.morph_delta_count)
                && usize::try_from(target.primitive)
                    .ok()
                    .and_then(|index| primitives.get(index))
                    .is_some_and(|primitive| {
                        primitive.morph_target_offset <= target_index
                            && primitive
                                .morph_target_offset
                                .checked_add(primitive.morph_target_count)
                                .is_some_and(|end| target_index < end)
                    })
        })?
        else {
            return Ok(None);
        };
        if !valid {
            return Ok(None);
        }
        delta_cursor = match delta_cursor.checked_add(target.delta_count) {
            Some(cursor) => cursor,
            None => return Ok(None),
        };
    }
    if delta_cursor != counts.morph_delta_count {
        return Ok(None);
    }

    for primitive in primitives.iter().copied() {
        let Some(valid) = session.run_validation_unit(|| {
            primitive
                .morph_target_offset
                .checked_add(primitive.morph_target_count)
                .is_some_and(|end| end <= counts.morph_target_count)
        })?
        else {
            return Ok(None);
        };
        if !valid {
            return Ok(None);
        }
    }

    let mut maximum_uniqueness_count = 0_usize;
    let mut processed_orders = 0_usize;
    for primitive in primitives
        .iter()
        .copied()
        .filter(|primitive| primitive.influences_present)
    {
        let Some(offset_range) = checked_range(
            primitive.row_offset_offset,
            primitive.row_offset_count,
            influence_row_offsets.len(),
        ) else {
            return Ok(None);
        };
        let offsets = &influence_row_offsets[offset_range];
        for pair in offsets.windows(2) {
            let (Ok(start), Ok(end)) = (usize::try_from(pair[0]), usize::try_from(pair[1])) else {
                return Ok(None);
            };
            let Some(width) = end.checked_sub(start) else {
                return Ok(None);
            };
            maximum_uniqueness_count = maximum_uniqueness_count.max(width);
            let Some(mut group) = uniqueness.begin_group(width) else {
                return Ok(None);
            };
            for local_index in start..end {
                let Some(global_index) = usize::try_from(primitive.influence_offset)
                    .ok()
                    .and_then(|offset| offset.checked_add(local_index))
                else {
                    return Ok(None);
                };
                let Some(order) = source_orders.get(global_index).copied() else {
                    return Ok(None);
                };
                let Some(valid) = session.run_validation_unit(|| group.mark(order).is_some())?
                else {
                    return Ok(None);
                };
                if !valid {
                    return Ok(None);
                }
                processed_orders = match processed_orders.checked_add(1) {
                    Some(count) => count,
                    None => return Ok(None),
                };
            }
        }
    }
    if processed_orders != influence_count {
        return Ok(None);
    }

    let Ok(logical_mesh_count) = usize::try_from(counts.logical_morph_count) else {
        return Ok(None);
    };
    let (Some(geometry_logical_meshes), Some(logical_morphs), Some(palette_maxima)) = (
        geometry.logical_meshes.get(..logical_mesh_count),
        logical_morphs.get(..logical_mesh_count),
        palette_maxima.get_mut(..logical_mesh_count),
    ) else {
        return Ok(None);
    };
    let mut geometry_primitive_cursor = 0_u32;
    for (logical_mesh_index, geometry_logical_mesh) in
        geometry_logical_meshes.iter().copied().enumerate()
    {
        let Some(valid) = session.run_validation_unit(|| {
            geometry_logical_mesh.primitive_offset == geometry_primitive_cursor
                && geometry_logical_mesh
                    .primitive_offset
                    .checked_add(geometry_logical_mesh.primitive_count)
                    .is_some_and(|end| end <= counts.primitive_count)
                && logical_morphs.get(logical_mesh_index).is_some()
        })?
        else {
            return Ok(None);
        };
        if !valid {
            return Ok(None);
        }
        palette_maxima[logical_mesh_index] = None;
        geometry_primitive_cursor =
            match geometry_primitive_cursor.checked_add(geometry_logical_mesh.primitive_count) {
                Some(cursor) => cursor,
                None => return Ok(None),
            };
    }
    if geometry_primitive_cursor != counts.primitive_count {
        return Ok(None);
    }

    for (logical_mesh_index, geometry_logical_mesh) in
        geometry_logical_meshes.iter().copied().enumerate()
    {
        let Some(range) = checked_range(
            geometry_logical_mesh.primitive_offset,
            geometry_logical_mesh.primitive_count,
            primitives.len(),
        ) else {
            return Ok(None);
        };
        for primitive in primitives[range].iter().copied() {
            let Some(valid) = session.run_validation_unit(|| {
                logical_morphs
                    .get(logical_mesh_index)
                    .is_some_and(|logical| primitive.morph_target_count == logical.definition_count)
            })?
            else {
                return Ok(None);
            };
            if !valid {
                return Ok(None);
            }
            let Some(influence_range) = checked_range(
                primitive.influence_offset,
                primitive.influence_count,
                influence_joints.len(),
            ) else {
                return Ok(None);
            };
            for joint in influence_joints[influence_range].iter().copied() {
                let Some(()) = session.run_validation_unit(|| {
                    record_palette_joint(&mut palette_maxima[logical_mesh_index], joint)
                })?
                else {
                    return Ok(None);
                };
            }
        }
    }

    for (instance_index, instance) in instances.iter().copied().enumerate() {
        let Some(valid) = session.run_validation_unit(|| {
            let Some(logical_mesh) = geometry
                .mesh_instances
                .get(instance_index)
                .and_then(|projection| usize::try_from(projection.logical_mesh).ok())
            else {
                return false;
            };
            if !instance.skin_present {
                return true;
            }
            let Some(skin) = usize::try_from(instance.skin)
                .ok()
                .and_then(|index| skins.get(index))
            else {
                return false;
            };
            palette_maxima.get(logical_mesh).is_some_and(|maximum| {
                palette_fits_skin(*maximum, StoredPresence::Present, skin.joint_count)
            })
        })?
        else {
            return Ok(None);
        };
        if !valid {
            return Ok(None);
        }
    }
    Ok(Some(maximum_uniqueness_count))
}

/// Validates the complete canonical deformation graph through typed, exactly metered visits.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn validate_deformation_graph_canonical_metered<
    'session,
    'storage,
    'control,
    'operation,
    O,
    PP,
    U,
>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: CanonicalValidationMember<'session, RecordBlock, DeformationDirectorySchema>,
    dependencies: DeformationValidationDependenciesMetered<'session, '_>,
    scratch: &mut DeformationValidationScratch<'_>,
) -> Result<Option<DeformationValidatedTypedShape<'session>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some((members, counts, payload_block_count)) =
        validate_deformation_directory_metered(session, directory)?
    else {
        return Ok(None);
    };
    let Some(requirements) = DeformationValidationScratchRequirements::try_new(
        counts.skin_count,
        counts.morph_definition_count,
        counts.logical_morph_count,
        counts.morph_delta_count,
        counts.morph_target_count,
        counts.primitive_count,
        counts.mesh_instance_count,
        counts.influence_row_offset_count,
        counts.influence_count,
    ) else {
        return Ok(None);
    };
    if !scratch.validate_capacity(requirements, counts.logical_morph_count) {
        return Ok(None);
    }
    let geometry_presence_valid = [
        (
            counts.logical_morph_count,
            dependencies.logical_meshes.is_some(),
            dependencies.geometry.logical_meshes.len(),
        ),
        (
            counts.primitive_count,
            dependencies.primitives.is_some(),
            dependencies.geometry.primitives.len(),
        ),
        (
            counts.mesh_instance_count,
            dependencies.mesh_instances.is_some(),
            dependencies.geometry.mesh_instances.len(),
        ),
    ]
    .into_iter()
    .all(|(count, member_present, projection_len)| {
        usize::try_from(count).is_ok_and(|expected_len| {
            projection_len == expected_len && member_present == (count != 0)
        })
    });
    if !geometry_presence_valid || dependencies.node_count == 0 {
        return Ok(None);
    }

    let DeformationValidationScratch {
        skins,
        morph_definitions,
        logical_morphs,
        morph_deltas,
        morph_targets,
        primitives,
        mesh_instances,
        influence_row_offsets,
        influence_joints,
        influence_source_orders,
        palette_maxima,
        uniqueness_marks,
    } = scratch;
    let (Some(skins), Some(morph_definitions), Some(logical_morphs)) = (
        skins.get_mut(..requirements.skins),
        morph_definitions.get_mut(..requirements.morph_definitions),
        logical_morphs.get_mut(..requirements.logical_morphs),
    ) else {
        return Ok(None);
    };
    let (Some(morph_deltas), Some(morph_targets), Some(primitives), Some(mesh_instances)) = (
        morph_deltas.get_mut(..requirements.morph_deltas),
        morph_targets.get_mut(..requirements.morph_targets),
        primitives.get_mut(..requirements.primitives),
        mesh_instances.get_mut(..requirements.mesh_instances),
    ) else {
        return Ok(None);
    };
    if !validate_skins_metered(
        session,
        members,
        counts,
        dependencies.nodes,
        dependencies.node_count,
        skins,
    )? || !validate_morph_definitions_metered(session, members, counts, morph_definitions)?
        || !validate_primitives_metered(
            session,
            members,
            counts,
            dependencies.geometry.primitives,
            primitives,
        )?
    {
        return Ok(None);
    }
    let mut uniqueness = DeformationUniquenessScratch::new(uniqueness_marks);
    let Some(logical_morph_maximum) = validate_logical_morphs_metered(
        session,
        members,
        counts,
        logical_morphs,
        morph_definitions,
        &mut uniqueness,
    )?
    else {
        return Ok(None);
    };
    if !validate_morph_targets_metered(session, members, counts, primitives, morph_targets)?
        || !validate_morph_deltas_metered(
            session,
            members,
            counts,
            morph_targets,
            dependencies.geometry.primitives,
            morph_deltas,
        )?
        || !validate_mesh_instances_metered(
            session,
            members,
            counts,
            dependencies.geometry.mesh_instances,
            logical_morphs,
            mesh_instances,
        )?
    {
        return Ok(None);
    }
    if !validate_deformation_payloads_metered(
        session,
        members,
        counts,
        dependencies.node_count,
        skins,
        morph_definitions,
        logical_morphs,
        morph_deltas,
        primitives,
        influence_row_offsets,
        influence_joints,
        influence_source_orders,
    )? {
        return Ok(None);
    }
    let Some(algorithm_maximum) = validate_deformation_algorithms_metered(
        session,
        counts,
        dependencies.geometry,
        skins,
        logical_morphs,
        morph_targets,
        primitives,
        mesh_instances,
        influence_row_offsets,
        influence_joints,
        influence_source_orders,
        palette_maxima,
        &mut uniqueness,
    )?
    else {
        return Ok(None);
    };

    Ok(Some(DeformationValidatedTypedShape {
        members,
        payload_block_count,
        skin_count: counts.skin_count,
        logical_morph_count: counts.logical_morph_count,
        morph_definition_count: counts.morph_definition_count,
        primitive_count: counts.primitive_count,
        morph_target_count: counts.morph_target_count,
        morph_delta_count: counts.morph_delta_count,
        mesh_instance_count: counts.mesh_instance_count,
        maximum_uniqueness_count: logical_morph_maximum.max(algorithm_maximum),
    }))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_deformation_graph<R, NR>(
    session: &mut StoreValidationSession<'_, '_>,
    deformation_ref: CanonicalRecordRef<DeformationDirectorySchema>,
    dependencies: DeformationValidationDependencies,
    expected_nodes: &ValidatedNodeBatch<'_, NR>,
    expected: &ValidatedDeformationBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<DeformationValidatedShape> {
    let fresh = FreshDeformationEquivalence {
        nodes: expected_nodes,
        deformation: expected,
    };
    validate_deformation_graph_impl(
        session,
        deformation_ref,
        dependencies,
        Some(&fresh),
        None,
        None,
        observe_work,
    )
}

pub(super) fn validate_deformation_graph_canonical(
    session: &mut StoreValidationSession<'_, '_>,
    deformation_ref: CanonicalRecordRef<DeformationDirectorySchema>,
    dependencies: DeformationValidationDependencies,
    palette_scratch: &mut [Option<u32>],
    uniqueness_scratch: &mut [u64],
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<DeformationValidatedShape> {
    validate_deformation_graph_impl::<(), ()>(
        session,
        deformation_ref,
        dependencies,
        None,
        Some(palette_scratch),
        Some(uniqueness_scratch),
        observe_work,
    )
}

#[allow(clippy::too_many_lines)]
fn validate_deformation_graph_impl<R, NR>(
    session: &mut StoreValidationSession<'_, '_>,
    deformation_ref: CanonicalRecordRef<DeformationDirectorySchema>,
    dependencies: DeformationValidationDependencies,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    palette_scratch: Option<&mut [Option<u32>]>,
    uniqueness_scratch: Option<&mut [u64]>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<DeformationValidatedShape> {
    let mut uniqueness_scratch = uniqueness_scratch.map(DeformationUniquenessScratch::new);
    observe_work(1)?;
    let directory = session.canonical_records(deformation_ref)?;
    if directory.rows() != 1 || directory.fields() != DEFORMATION_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, deformation_directory_field::SCHEMA_REVISION),
        DEFORMATION_SCHEMA_REVISION,
    )?;

    // These three counts qualify the cross-domain dependency shapes before any
    // deformation-owned reference is followed, preserving the fresh validation order.
    let dependency_logical_morph_count =
        directory.scalar::<u32>(0, deformation_directory_field::LOGICAL_MORPH_COUNT)?;
    let dependency_primitive_count =
        directory.scalar::<u32>(0, deformation_directory_field::PRIMITIVE_DEFORMATION_COUNT)?;
    let dependency_mesh_instance_count =
        directory.scalar::<u32>(0, deformation_directory_field::MESH_INSTANCE_COUNT)?;
    let dependency_views = validate_dependencies(
        session,
        dependencies,
        dependency_logical_morph_count,
        dependency_primitive_count,
        dependency_mesh_instance_count,
        fresh,
    )?;

    let (skin_count, skins) = optional_record_ref::<DeformationSkinTableSchema>(
        session,
        directory,
        deformation_directory_field::SKIN_COUNT,
        deformation_directory_field::SKINS,
        DEFORMATION_SKIN_FIELD_COUNT,
    )?;
    require_fresh_count(
        skin_count,
        fresh.map(|fresh| fresh.deformation.skin_count()),
    )?;
    let (joint_node_count, joint_nodes) =
        optional_sequence_ref::<u32, DeformationSkinJointNodeSchema>(
            session,
            directory,
            deformation_directory_field::JOINT_NODE_COUNT,
            deformation_directory_field::JOINT_NODES,
        )?;
    require_fresh_count(
        joint_node_count,
        fresh.map(|fresh| fresh.deformation.joint_node_count()),
    )?;
    let (inverse_bind_scalar_count, inverse_bind_matrices) =
        optional_sequence_ref::<f32, DeformationInverseBindMatrixSchema>(
            session,
            directory,
            deformation_directory_field::INVERSE_BIND_SCALAR_COUNT,
            deformation_directory_field::INVERSE_BIND_MATRICES,
        )?;
    require_fresh_count(
        inverse_bind_scalar_count,
        fresh.map(|fresh| fresh.deformation.inverse_bind_scalar_count()),
    )?;
    let (logical_morph_count, logical_morphs) =
        optional_record_ref::<DeformationLogicalMorphTableSchema>(
            session,
            directory,
            deformation_directory_field::LOGICAL_MORPH_COUNT,
            deformation_directory_field::LOGICAL_MORPHS,
            DEFORMATION_LOGICAL_MORPH_FIELD_COUNT,
        )?;
    require_scalar(Some(logical_morph_count), dependency_logical_morph_count)?;
    require_fresh_count(
        logical_morph_count,
        fresh.map(|fresh| fresh.deformation.logical_morph_count()),
    )?;
    let (primitive_count, primitives) = optional_record_ref::<DeformationPrimitiveTableSchema>(
        session,
        directory,
        deformation_directory_field::PRIMITIVE_DEFORMATION_COUNT,
        deformation_directory_field::PRIMITIVE_DEFORMATIONS,
        DEFORMATION_PRIMITIVE_FIELD_COUNT,
    )?;
    require_scalar(Some(primitive_count), dependency_primitive_count)?;
    require_fresh_count(
        primitive_count,
        fresh.map(|fresh| fresh.deformation.primitive_count()),
    )?;
    let (influence_row_offset_count, influence_row_offsets) =
        optional_sequence_ref::<u32, DeformationInfluenceRowOffsetSchema>(
            session,
            directory,
            deformation_directory_field::INFLUENCE_ROW_OFFSET_COUNT,
            deformation_directory_field::INFLUENCE_ROW_OFFSETS,
        )?;
    require_fresh_count(
        influence_row_offset_count,
        fresh.map(|fresh| fresh.deformation.influence_row_offset_count()),
    )?;
    let (influence_count, influence_joints) =
        optional_sequence_ref::<u32, DeformationInfluenceJointSchema>(
            session,
            directory,
            deformation_directory_field::INFLUENCE_COUNT,
            deformation_directory_field::INFLUENCE_JOINTS,
        )?;
    require_fresh_count(
        influence_count,
        fresh.map(|fresh| fresh.deformation.influence_count()),
    )?;
    let influence_weights =
        optional_sequence_ref_same_count::<f32, DeformationInfluenceWeightSchema>(
            session,
            directory,
            deformation_directory_field::INFLUENCE_WEIGHTS,
            influence_count,
        )?;
    let influence_source_orders =
        optional_sequence_ref_same_count::<u32, DeformationInfluenceSourceOrderSchema>(
            session,
            directory,
            deformation_directory_field::INFLUENCE_SOURCE_ORDERS,
            influence_count,
        )?;
    let (morph_target_count, morph_targets) =
        optional_record_ref::<DeformationMorphTargetTableSchema>(
            session,
            directory,
            deformation_directory_field::MORPH_TARGET_COUNT,
            deformation_directory_field::MORPH_TARGETS,
            DEFORMATION_MORPH_TARGET_FIELD_COUNT,
        )?;
    require_fresh_count(
        morph_target_count,
        fresh.map(|fresh| fresh.deformation.morph_target_count()),
    )?;
    let (morph_delta_count, morph_deltas) = optional_record_ref::<DeformationMorphDeltaTableSchema>(
        session,
        directory,
        deformation_directory_field::MORPH_DELTA_COUNT,
        deformation_directory_field::MORPH_DELTAS,
        DEFORMATION_MORPH_DELTA_FIELD_COUNT,
    )?;
    require_fresh_count(
        morph_delta_count,
        fresh.map(|fresh| fresh.deformation.morph_delta_count()),
    )?;
    let (morph_default_count, morph_defaults) =
        optional_sequence_ref::<f32, DeformationMorphDefaultWeightSchema>(
            session,
            directory,
            deformation_directory_field::MORPH_DEFAULT_COUNT,
            deformation_directory_field::MORPH_DEFAULTS,
        )?;
    require_fresh_count(
        morph_default_count,
        fresh.map(|fresh| fresh.deformation.morph_default_weight_count()),
    )?;
    let (morph_value_count, morph_values) =
        optional_sequence_ref::<f32, DeformationMorphValueSchema>(
            session,
            directory,
            deformation_directory_field::MORPH_VALUE_COUNT,
            deformation_directory_field::MORPH_VALUES,
        )?;
    require_fresh_count(
        morph_value_count,
        fresh.map(|fresh| fresh.deformation.morph_value_count()),
    )?;
    let (morph_sparse_index_count, morph_sparse_indices) =
        optional_sequence_ref::<u32, DeformationMorphSparseIndexSchema>(
            session,
            directory,
            deformation_directory_field::MORPH_SPARSE_INDEX_COUNT,
            deformation_directory_field::MORPH_SPARSE_INDICES,
        )?;
    require_fresh_count(
        morph_sparse_index_count,
        fresh.map(|fresh| fresh.deformation.morph_sparse_index_count()),
    )?;
    let (mesh_instance_count, mesh_instances) =
        optional_record_ref::<DeformationMeshInstanceTableSchema>(
            session,
            directory,
            deformation_directory_field::MESH_INSTANCE_COUNT,
            deformation_directory_field::MESH_INSTANCES,
            DEFORMATION_MESH_INSTANCE_FIELD_COUNT,
        )?;
    require_scalar(Some(mesh_instance_count), dependency_mesh_instance_count)?;
    require_fresh_count(
        mesh_instance_count,
        fresh.map(|fresh| fresh.deformation.mesh_instance_count()),
    )?;
    let (instance_morph_weight_count, instance_morph_weights) =
        optional_sequence_ref::<f32, DeformationInstanceMorphWeightSchema>(
            session,
            directory,
            deformation_directory_field::INSTANCE_MORPH_WEIGHT_COUNT,
            deformation_directory_field::INSTANCE_MORPH_WEIGHTS,
        )?;
    require_fresh_count(
        instance_morph_weight_count,
        fresh.map(|fresh| fresh.deformation.instance_morph_weight_count()),
    )?;
    let skin_correction_matrices =
        optional_sequence_ref::<f32, DeformationSkinCorrectionMatrixSchema>(
            session,
            directory,
            deformation_directory_field::SKIN_CORRECTION_SCALAR_COUNT,
            deformation_directory_field::SKIN_CORRECTION_MATRICES,
        )?;
    let (skin_correction_scalar_count, skin_correction_matrices) = skin_correction_matrices;
    require_fresh_count(
        skin_correction_scalar_count,
        fresh.map(|fresh| fresh.deformation.skin_correction_scalar_count()),
    )?;
    let (morph_definition_count, morph_definitions) =
        optional_record_ref::<DeformationMorphDefinitionTableSchema>(
            session,
            directory,
            deformation_directory_field::MORPH_DEFINITION_COUNT,
            deformation_directory_field::MORPH_DEFINITIONS,
            DEFORMATION_MORPH_DEFINITION_FIELD_COUNT,
        )?;
    require_fresh_count(
        morph_definition_count,
        fresh.map(|fresh| fresh.deformation.morph_definition_count()),
    )?;
    let (morph_name_byte_count, morph_names) = optional_utf8_ref::<DeformationMorphNameSchema>(
        session,
        directory,
        deformation_directory_field::MORPH_NAME_BYTE_COUNT,
        deformation_directory_field::MORPH_NAMES,
    )?;
    require_fresh_count(
        morph_name_byte_count,
        fresh.map(|fresh| fresh.deformation.morph_name_byte_count()),
    )?;
    let (skin_name_byte_count, skin_names) = optional_utf8_ref::<DeformationSkinNameSchema>(
        session,
        directory,
        deformation_directory_field::SKIN_NAME_BYTE_COUNT,
        deformation_directory_field::SKIN_NAMES,
    )?;
    require_fresh_count(
        skin_name_byte_count,
        fresh.map(|fresh| fresh.deformation.skin_name_byte_count()),
    )?;

    let payload_block_count = [
        joint_nodes.is_some(),
        inverse_bind_matrices.is_some(),
        skin_correction_matrices.is_some(),
        skin_names.is_some(),
        influence_row_offsets.is_some(),
        influence_joints.is_some(),
        influence_weights.is_some(),
        influence_source_orders.is_some(),
        morph_defaults.is_some(),
        morph_names.is_some(),
        morph_values.is_some(),
        morph_sparse_indices.is_some(),
        instance_morph_weights.is_some(),
    ]
    .into_iter()
    .try_fold(0_u32, |count, present| {
        count.checked_add(u32::from(present))
    })?;
    require_fresh_count(
        payload_block_count,
        fresh.map(|fresh| fresh.deformation.payload_block_count()),
    )?;

    let skin_view = validate_skins(
        session,
        skins,
        joint_nodes,
        inverse_bind_matrices,
        skin_correction_matrices,
        skin_names,
        dependencies.nodes.into_store(),
        dependency_views.nodes,
        skin_count,
        joint_node_count,
        inverse_bind_scalar_count,
        skin_correction_scalar_count,
        skin_name_byte_count,
        fresh,
        observe_work,
    )?;
    let morph_definition_view = validate_morph_definitions(
        session,
        morph_definitions,
        morph_names,
        morph_definition_count,
        morph_name_byte_count,
        logical_morph_count,
        fresh,
        observe_work,
    )?;
    let (logical_morph_view, logical_morph_maximum_uniqueness_count) = validate_logical_morphs(
        session,
        logical_morphs,
        morph_definitions,
        morph_definition_view,
        morph_defaults,
        logical_morph_count,
        morph_definition_count,
        morph_default_count,
        fresh,
        uniqueness_scratch.as_mut(),
        observe_work,
    )?;
    let morph_target_view = validate_morph_deltas(
        session,
        morph_deltas,
        morph_values,
        morph_sparse_indices,
        morph_targets,
        dependency_views.primitives,
        morph_delta_count,
        morph_value_count,
        morph_sparse_index_count,
        fresh,
        observe_work,
    )?;
    let primitive_view = validate_morph_targets(
        session,
        morph_targets,
        morph_target_view,
        morph_deltas,
        primitives,
        morph_target_count,
        morph_delta_count,
        fresh,
        observe_work,
    )?;
    let primitive_views = validate_primitives(
        session,
        primitives,
        primitive_view,
        influence_row_offsets,
        influence_joints,
        influence_weights,
        influence_source_orders,
        morph_targets,
        dependency_views.primitives,
        primitive_count,
        influence_row_offset_count,
        influence_count,
        morph_target_count,
        fresh,
        uniqueness_scratch.as_mut(),
        observe_work,
    )?;
    let mesh_instance_view = validate_mesh_instances(
        session,
        mesh_instances,
        skins,
        instance_morph_weights,
        logical_morph_view,
        dependency_views.mesh_instances,
        skin_count,
        mesh_instance_count,
        instance_morph_weight_count,
        fresh,
        observe_work,
    )?;
    if fresh.is_none() {
        validate_canonical_cross_domain_semantics(
            dependency_views.logical_meshes,
            dependency_views.mesh_instances,
            logical_morph_view,
            primitive_views.primitives,
            mesh_instance_view,
            skin_view,
            primitive_views.influence_joints,
            palette_scratch?,
            observe_work,
        )?;
    } else if palette_scratch.is_some() {
        return None;
    }

    Some(DeformationValidatedShape {
        refs: DeformationValidatedRefs {
            skins: skins.map(CanonicalBlockRef::from_store),
            logical_morphs: logical_morphs.map(CanonicalBlockRef::from_store),
            morph_definitions: morph_definitions.map(CanonicalBlockRef::from_store),
            primitives: primitives.map(CanonicalBlockRef::from_store),
            morph_targets: morph_targets.map(CanonicalBlockRef::from_store),
            morph_deltas: morph_deltas.map(CanonicalBlockRef::from_store),
            mesh_instances: mesh_instances.map(CanonicalBlockRef::from_store),
            joint_nodes: joint_nodes.map(CanonicalBlockRef::from_store),
            inverse_bind_matrices: inverse_bind_matrices.map(CanonicalBlockRef::from_store),
            skin_correction_matrices: skin_correction_matrices.map(CanonicalBlockRef::from_store),
            skin_names: skin_names.map(CanonicalBlockRef::from_store),
            influence_row_offsets: influence_row_offsets.map(CanonicalBlockRef::from_store),
            influence_joints: influence_joints.map(CanonicalBlockRef::from_store),
            influence_weights: influence_weights.map(CanonicalBlockRef::from_store),
            influence_source_orders: influence_source_orders.map(CanonicalBlockRef::from_store),
            morph_defaults: morph_defaults.map(CanonicalBlockRef::from_store),
            morph_names: morph_names.map(CanonicalBlockRef::from_store),
            morph_values: morph_values.map(CanonicalBlockRef::from_store),
            morph_sparse_indices: morph_sparse_indices.map(CanonicalBlockRef::from_store),
            instance_morph_weights: instance_morph_weights.map(CanonicalBlockRef::from_store),
        },
        payload_block_count,
        skin_count,
        logical_morph_count,
        morph_definition_count,
        primitive_count,
        morph_target_count,
        morph_delta_count,
        mesh_instance_count,
        maximum_uniqueness_count: logical_morph_maximum_uniqueness_count
            .max(primitive_views.maximum_uniqueness_count),
    })
}

fn validate_dependencies<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    dependencies: DeformationValidationDependencies,
    logical_morph_count: u32,
    primitive_count: u32,
    mesh_instance_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
) -> Option<DeformationDependencyViews<'storage>> {
    let nodes = session.canonical_records(dependencies.nodes.into_store())?;
    if nodes.fields() != NODE_SCENE_NODE_FIELD_COUNT {
        return None;
    }
    if let Some(fresh) = fresh
        && nodes.rows() != usize::try_from(fresh.nodes.canonical_node_count()).ok()?
    {
        return None;
    }
    let logical_meshes = validate_optional_dependency(
        session,
        dependencies.logical_meshes,
        logical_morph_count,
        fresh.map(|fresh| fresh.deformation.logical_morph_count()),
        GEOMETRY_LOGICAL_MESH_FIELD_COUNT,
    )?;
    let primitives = validate_optional_dependency(
        session,
        dependencies.primitives,
        primitive_count,
        fresh.map(|fresh| fresh.deformation.primitive_count()),
        GEOMETRY_PRIMITIVE_FIELD_COUNT,
    )?;
    let mesh_instances = validate_optional_dependency(
        session,
        dependencies.mesh_instances,
        mesh_instance_count,
        fresh.map(|fresh| fresh.deformation.mesh_instance_count()),
        GEOMETRY_MESH_INSTANCE_FIELD_COUNT,
    )?;
    Some(DeformationDependencyViews {
        nodes,
        logical_meshes,
        primitives,
        mesh_instances,
    })
}

fn validate_optional_dependency<'storage, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalBlockRef<Tag>>,
    canonical_count: u32,
    fresh_count: Option<u32>,
    expected_fields: usize,
) -> Option<Option<CanonicalRecordView<'storage>>>
where
    Tag: crate::canonical_scene::store::schema::CanonicalSchema<
            StoreKind = baozi_operation::RecordBlock,
        >,
{
    let view = match (canonical_count, reference) {
        (0, None) => Some(None),
        (0, Some(_)) | (_, None) => None,
        (_, Some(reference)) => {
            let table = session.canonical_records(reference.into_store())?;
            if table.rows() != usize::try_from(canonical_count).ok()?
                || table.fields() != expected_fields
            {
                return None;
            }
            Some(Some(table))
        }
    }?;
    require_fresh_count(canonical_count, fresh_count)?;
    Some(view)
}

fn optional_record_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
    expected_fields: usize,
) -> Option<(u32, Option<CanonicalRecordRef<Tag>>)>
where
    Tag: Copy + Send + Sync + 'static,
{
    let count = directory.scalar::<u32>(0, count_field)?;
    if count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some((count, None))
    } else {
        let reference = directory.record_ref::<Tag>(0, reference_field)?;
        let table = session.canonical_records(reference)?;
        if table.rows() != usize::try_from(count).ok()? || table.fields() != expected_fields {
            return None;
        }
        Some((count, Some(reference)))
    }
}

fn optional_sequence_ref<T, Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
) -> Option<(u32, Option<CanonicalSequenceRef<T, Tag>>)>
where
    T: StoredScalar,
    Tag: Copy + Send + Sync + 'static,
{
    let count = directory.scalar::<u32>(0, count_field)?;
    optional_sequence_ref_same_count(session, directory, reference_field, count)
        .map(|reference| (count, reference))
}

fn optional_sequence_ref_same_count<T, Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    reference_field: usize,
    expected_count: u32,
) -> Option<Option<CanonicalSequenceRef<T, Tag>>>
where
    T: StoredScalar,
    Tag: Copy + Send + Sync + 'static,
{
    if expected_count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some(None)
    } else {
        let reference = directory.sequence_ref::<T, Tag>(0, reference_field)?;
        let sequence = session.canonical_sequence(reference)?;
        (sequence.len() == usize::try_from(expected_count).ok()?).then_some(Some(reference))
    }
}

fn optional_utf8_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
) -> Option<(u32, Option<CanonicalUtf8Ref<Tag>>)>
where
    Tag: Copy + Send + Sync + 'static,
{
    let count = directory.scalar::<u32>(0, count_field)?;
    if count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some((count, None))
    } else {
        let reference = directory.utf8_ref::<Tag>(0, reference_field)?;
        let payload = session.canonical_utf8(reference)?;
        (payload.len() == usize::try_from(count).ok()?).then_some((count, Some(reference)))
    }
}

fn require_fresh_count(actual: u32, expected: Option<u32>) -> Option<()> {
    expected
        .is_none_or(|expected| actual == expected)
        .then_some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_skins<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<DeformationSkinTableSchema>>,
    joint_nodes: Option<CanonicalSequenceRef<u32, DeformationSkinJointNodeSchema>>,
    inverse_bind_matrices: Option<CanonicalSequenceRef<f32, DeformationInverseBindMatrixSchema>>,
    correction_matrices: Option<CanonicalSequenceRef<f32, DeformationSkinCorrectionMatrixSchema>>,
    names: Option<CanonicalUtf8Ref<DeformationSkinNameSchema>>,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    nodes: CanonicalRecordView<'storage>,
    skin_count: u32,
    joint_node_count: u32,
    inverse_bind_scalar_count: u32,
    correction_scalar_count: u32,
    name_byte_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<Option<CanonicalRecordView<'storage>>> {
    let Some(reference) = reference else {
        return (skin_count == 0).then_some(None);
    };
    let table = session.canonical_records(reference)?;
    let joint_values = match joint_nodes {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let inverse_values = match inverse_bind_matrices {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let correction_values = match correction_matrices {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let name_values = match names {
        Some(reference) => Some(session.canonical_utf8(reference)?),
        None => None,
    };
    let mapping = fresh.map(|fresh| fresh.nodes.canonical_index_by_local());
    let mut joint_cursor = 0_u32;
    let mut inverse_cursor = 0_u32;
    let mut correction_cursor = 0_u32;
    let mut name_cursor = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        let expected_skin = match fresh {
            Some(fresh) => Some(fresh.deformation.skin(row)?),
            None => None,
        };
        let skin_id = u32::try_from(row).ok()?;
        let stored_joint_offset = table.scalar::<u32>(row, deformation_skin_field::JOINT_OFFSET)?;
        let joint_count = table.scalar::<u32>(row, deformation_skin_field::JOINT_COUNT)?;
        let stored_inverse_offset =
            table.scalar::<u32>(row, deformation_skin_field::INVERSE_BIND_OFFSET)?;
        let inverse_count = table.scalar::<u32>(row, deformation_skin_field::INVERSE_BIND_COUNT)?;
        let inverse_scalar_count = inverse_count.checked_mul(MATRIX_SCALAR_COUNT)?;
        if joint_count != inverse_count
            || stored_joint_offset != joint_cursor
            || stored_inverse_offset != inverse_cursor
        {
            return None;
        }
        if let Some(skin) = expected_skin.as_ref() {
            let authored = skin.authored();
            if skin.joint_offset() != joint_cursor
                || skin.joint_count() != joint_count
                || skin.inverse_bind_scalar_offset() != inverse_cursor
                || usize::try_from(joint_count).ok()? != authored.joints().len()
                || usize::try_from(inverse_count).ok()? != authored.inverse_bind_matrices().len()
            {
                return None;
            }
        }
        require_scalar(
            table.scalar::<u32>(row, deformation_skin_field::SKIN),
            skin_id,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_skin_field::JOINT_OFFSET),
            joint_cursor,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_skin_field::JOINT_COUNT),
            joint_count,
        )?;
        require_optional_sequence_reference(
            table,
            row,
            deformation_skin_field::JOINTS,
            joint_count != 0,
            joint_nodes,
        )?;
        if joint_count != 0 {
            let stored = joint_values?.as_slice();
            let range = checked_range(joint_cursor, joint_count, stored.len())?;
            let stored = stored.get(range)?;
            observe_work(u64::from(joint_count))?;
            if stored
                .iter()
                .any(|joint| !is_authored_node_index(*joint, nodes.rows()))
            {
                return None;
            }
            if let Some(skin) = expected_skin.as_ref() {
                for (stored, authored) in stored.iter().zip(skin.authored().joints()) {
                    let local = usize::try_from(authored.index()).ok()?;
                    if *stored != *mapping?.get(local)? {
                        return None;
                    }
                }
            }
        }

        require_scalar(
            table.scalar::<u32>(row, deformation_skin_field::INVERSE_BIND_OFFSET),
            inverse_cursor,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_skin_field::INVERSE_BIND_COUNT),
            inverse_count,
        )?;
        require_optional_sequence_reference(
            table,
            row,
            deformation_skin_field::INVERSE_BINDS,
            inverse_count != 0,
            inverse_bind_matrices,
        )?;
        if inverse_scalar_count != 0 {
            let stored = inverse_values?.as_slice();
            let range = checked_range(inverse_cursor, inverse_scalar_count, stored.len())?;
            let stored = stored.get(range)?;
            observe_work(u64::from(inverse_scalar_count))?;
            validate_affine_matrices(stored)?;
            if let Some(skin) = expected_skin.as_ref() {
                let mut scalar_index = 0_usize;
                for matrix in skin.authored().inverse_bind_matrices() {
                    for column in matrix.cols {
                        for expected_scalar in column {
                            require_float32_bits(
                                stored.get(scalar_index).copied(),
                                expected_scalar,
                            )?;
                            scalar_index = scalar_index.checked_add(1)?;
                        }
                    }
                }
                if scalar_index != stored.len() {
                    return None;
                }
            }
        }

        let root = match require_presence(
            table.scalar::<u8>(row, deformation_skin_field::SKELETON_ROOT_PRESENCE),
        )? {
            StoredPresence::Absent => {
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::SKELETON_ROOT),
                    0_u32,
                )?;
                None
            }
            StoredPresence::Present => {
                let root = table.scalar::<u32>(row, deformation_skin_field::SKELETON_ROOT)?;
                if !is_authored_node_index(root, nodes.rows()) {
                    return None;
                }
                Some(root)
            }
        };
        if let Some(skin) = expected_skin.as_ref() {
            match (skin.authored().skeleton_root(), root) {
                (Presence::Absent, None) => {}
                (Presence::Present(expected), Some(root)) => {
                    let local = usize::try_from(expected.index()).ok()?;
                    require_scalar(Some(root), *mapping?.get(local)?)?;
                }
                _ => return None,
            }
        }
        require_optional_record_reference(
            table,
            row,
            deformation_skin_field::NODES,
            joint_count != 0 || root.is_some(),
            Some(nodes_ref),
        )?;

        let space_fields = [
            deformation_skin_field::INPUT_POSITION_SPACE,
            deformation_skin_field::INVERSE_BIND_FROM_SPACE,
            deformation_skin_field::INVERSE_BIND_TO_SPACE,
            deformation_skin_field::CURRENT_JOINT_FROM_SPACE,
            deformation_skin_field::CURRENT_JOINT_TO_SPACE,
            deformation_skin_field::OUTPUT_POSITION_SPACE,
        ];
        let canonical_spaces = [
            DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND,
            DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND,
            DEFORMATION_SPACE_JOINT_LOCAL_BIND,
            DEFORMATION_SPACE_JOINT_LOCAL,
            DEFORMATION_SPACE_SCENE,
            DEFORMATION_SPACE_MESH_INSTANCE_LOCAL,
        ];
        for (field, encoded) in space_fields.into_iter().zip(canonical_spaces) {
            require_scalar(table.scalar::<u8>(row, field), encoded)?;
        }
        if let Some(skin) = expected_skin.as_ref() {
            for (field, encoded) in
                space_fields
                    .into_iter()
                    .zip(encode_deformation_skin_matrix_spaces(
                        skin.authored().matrix_spaces(),
                    ))
            {
                require_scalar(table.scalar::<u8>(row, field), encoded)?;
            }
        }

        let correction_present =
            require_presence(table.scalar::<u8>(row, deformation_skin_field::CORRECTION_PRESENCE))?;
        match correction_present {
            StoredPresence::Absent => {
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::CORRECTION_FROM_SPACE),
                    0_u8,
                )?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::CORRECTION_TO_SPACE),
                    0_u8,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::CORRECTION_MATRIX_OFFSET),
                    0_u32,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::CORRECTION_MATRIX_COUNT),
                    0_u32,
                )?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::CORRECTION_MATRICES),
                    0_u8,
                )?;
            }
            StoredPresence::Present => {
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::CORRECTION_FROM_SPACE),
                    DEFORMATION_SPACE_LOGICAL_MESH_LOCAL_BIND,
                )?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_skin_field::CORRECTION_TO_SPACE),
                    DEFORMATION_SPACE_MESH_INSTANCE_LOCAL,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::CORRECTION_MATRIX_OFFSET),
                    correction_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_skin_field::CORRECTION_MATRIX_COUNT),
                    MATRIX_SCALAR_COUNT,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_skin_field::CORRECTION_MATRICES,
                    true,
                    correction_matrices,
                )?;
                let stored = correction_values?.as_slice();
                let range = checked_range(correction_cursor, MATRIX_SCALAR_COUNT, stored.len())?;
                let stored = stored.get(range)?;
                observe_work(u64::from(MATRIX_SCALAR_COUNT))?;
                validate_affine_matrices(stored)?;
                if let Some(skin) = expected_skin.as_ref() {
                    let correction = skin.authored().matrix_spaces().correction()?;
                    if skin.correction_matrix_scalar_offset() != correction_cursor
                        || skin.correction_matrix_scalar_count() != MATRIX_SCALAR_COUNT
                    {
                        return None;
                    }
                    require_scalar(
                        table.scalar::<u8>(row, deformation_skin_field::CORRECTION_FROM_SPACE),
                        encode_deformation_skin_space(correction.from()),
                    )?;
                    require_scalar(
                        table.scalar::<u8>(row, deformation_skin_field::CORRECTION_TO_SPACE),
                        encode_deformation_skin_space(correction.to()),
                    )?;
                    let mut scalar_index = 0_usize;
                    for column in correction.matrix().cols {
                        for expected_scalar in column {
                            require_float32_bits(
                                stored.get(scalar_index).copied(),
                                expected_scalar,
                            )?;
                            scalar_index = scalar_index.checked_add(1)?;
                        }
                    }
                    if scalar_index != stored.len() {
                        return None;
                    }
                }
                correction_cursor = correction_cursor.checked_add(MATRIX_SCALAR_COUNT)?;
            }
        }
        if let Some(skin) = expected_skin.as_ref() {
            match (
                skin.authored().matrix_spaces().correction(),
                correction_present,
            ) {
                (None, StoredPresence::Absent) => {
                    if skin.correction_matrix_scalar_offset() != 0
                        || skin.correction_matrix_scalar_count() != 0
                    {
                        return None;
                    }
                }
                (Some(_), StoredPresence::Present) => {}
                _ => return None,
            }
        }

        let name_present =
            require_presence(table.scalar::<u8>(row, deformation_skin_field::NAME_PRESENCE))?;
        let stored_name_offset = table.scalar::<u32>(row, deformation_skin_field::NAME_OFFSET)?;
        let stored_name_count = table.scalar::<u32>(row, deformation_skin_field::NAME_COUNT)?;
        match name_present {
            StoredPresence::Absent => {
                require_scalar(
                    Some(stored_name_offset.checked_add(stored_name_count)?),
                    0_u32,
                )?;
                require_scalar(table.scalar::<u8>(row, deformation_skin_field::NAMES), 0_u8)?;
            }
            StoredPresence::Present => {
                require_scalar(Some(stored_name_offset), name_cursor)?;
                require_optional_utf8_reference(
                    table,
                    row,
                    deformation_skin_field::NAMES,
                    stored_name_count != 0,
                    names,
                )?;
                let stored =
                    checked_utf8_range(name_values.unwrap_or(""), name_cursor, stored_name_count)?;
                if stored_name_count != 0 {
                    observe_work(u64::from(stored_name_count))?;
                }
                validate_entity_name_syntax(stored)?;
                if let Some(skin) = expected_skin.as_ref() {
                    let Presence::Present(name) = skin.authored().name() else {
                        return None;
                    };
                    if stored != name.as_str() {
                        return None;
                    }
                }
                name_cursor = name_cursor.checked_add(stored_name_count)?;
            }
        }
        if let Some(skin) = expected_skin.as_ref()
            && (skin.name_byte_offset() != stored_name_offset
                || skin.name_byte_count() != stored_name_count
                || matches!(skin.authored().name(), Presence::Present(_))
                    != (name_present == StoredPresence::Present))
        {
            return None;
        }

        joint_cursor = joint_cursor.checked_add(joint_count)?;
        inverse_cursor = inverse_cursor.checked_add(inverse_scalar_count)?;
    }
    (joint_cursor == joint_node_count
        && inverse_cursor == inverse_bind_scalar_count
        && correction_cursor == correction_scalar_count
        && name_cursor == name_byte_count)
        .then_some(Some(table))
}

#[allow(clippy::too_many_arguments)]
fn validate_morph_definitions<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<DeformationMorphDefinitionTableSchema>>,
    names: Option<CanonicalUtf8Ref<DeformationMorphNameSchema>>,
    definition_count: u32,
    name_byte_count: u32,
    logical_morph_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<Option<CanonicalRecordView<'storage>>> {
    let Some(reference) = reference else {
        return (definition_count == 0).then_some(None);
    };
    let table = session.canonical_records(reference)?;
    let name_values = match names {
        Some(reference) => Some(session.canonical_utf8(reference)?),
        None => None,
    };
    let mut name_cursor = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        let expected_definition = match fresh {
            Some(fresh) => Some(fresh.deformation.morph_definition(row)?),
            None => None,
        };
        require_scalar(
            table.scalar::<u32>(row, deformation_morph_definition_field::MORPH_DEFINITION),
            u32::try_from(row).ok()?,
        )?;
        let logical_mesh =
            table.scalar::<u32>(row, deformation_morph_definition_field::LOGICAL_MESH)?;
        let target_ordinal =
            table.scalar::<u32>(row, deformation_morph_definition_field::TARGET_ORDINAL)?;
        let stable_identity =
            table.scalar::<u32>(row, deformation_morph_definition_field::STABLE_IDENTITY)?;
        if logical_mesh >= logical_morph_count {
            return None;
        }
        if let Some(definition) = expected_definition.as_ref()
            && (logical_mesh != definition.logical_mesh()
                || target_ordinal != definition.target_ordinal()
                || stable_identity != definition.authored().identity().value())
        {
            return None;
        }
        let name_present = require_presence(
            table.scalar::<u8>(row, deformation_morph_definition_field::NAME_PRESENCE),
        )?;
        let name_offset =
            table.scalar::<u32>(row, deformation_morph_definition_field::NAME_OFFSET)?;
        let name_count =
            table.scalar::<u32>(row, deformation_morph_definition_field::NAME_COUNT)?;
        match name_present {
            StoredPresence::Absent => {
                require_scalar(Some(name_offset.checked_add(name_count)?), 0_u32)?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_morph_definition_field::NAMES),
                    0_u8,
                )?;
            }
            StoredPresence::Present => {
                require_scalar(Some(name_offset), name_cursor)?;
                require_optional_utf8_reference(
                    table,
                    row,
                    deformation_morph_definition_field::NAMES,
                    name_count != 0,
                    names,
                )?;
                let stored =
                    checked_utf8_range(name_values.unwrap_or(""), name_cursor, name_count)?;
                if name_count != 0 {
                    observe_work(u64::from(name_count))?;
                }
                validate_entity_name_syntax(stored)?;
                if let Some(definition) = expected_definition.as_ref() {
                    let Presence::Present(name) = definition.authored().name() else {
                        return None;
                    };
                    if stored != name.as_str() {
                        return None;
                    }
                }
                name_cursor = name_cursor.checked_add(name_count)?;
            }
        }
        if let Some(definition) = expected_definition.as_ref()
            && (definition.name_byte_offset() != name_offset
                || definition.name_byte_count() != name_count
                || matches!(definition.authored().name(), Presence::Present(_))
                    != (name_present == StoredPresence::Present))
        {
            return None;
        }
    }
    (name_cursor == name_byte_count).then_some(Some(table))
}

#[allow(clippy::too_many_arguments)]
fn validate_logical_morphs<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<DeformationLogicalMorphTableSchema>>,
    definitions: Option<CanonicalRecordRef<DeformationMorphDefinitionTableSchema>>,
    definition_values: Option<CanonicalRecordView<'storage>>,
    defaults: Option<CanonicalSequenceRef<f32, DeformationMorphDefaultWeightSchema>>,
    logical_morph_count: u32,
    definition_count: u32,
    default_weight_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    mut uniqueness_scratch: Option<&mut DeformationUniquenessScratch<'_>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<(Option<CanonicalRecordView<'storage>>, usize)> {
    let Some(reference) = reference else {
        return (logical_morph_count == 0).then_some((None, 0));
    };
    let table = session.canonical_records(reference)?;
    let default_values = match defaults {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let mut definition_cursor = 0_u32;
    let mut default_cursor = 0_u32;
    let mut maximum_uniqueness_count = 0_usize;
    for row in 0..table.rows() {
        observe_work(1)?;
        let expected_logical = match fresh {
            Some(fresh) => Some(fresh.deformation.logical_morph(row)?),
            None => None,
        };
        let stored_definition_offset =
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFINITION_OFFSET)?;
        let stored_definition_count =
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFINITION_COUNT)?;
        let stored_default_offset =
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFAULT_WEIGHT_OFFSET)?;
        let stored_default_count =
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFAULT_WEIGHT_COUNT)?;
        if stored_definition_offset != definition_cursor
            || stored_default_offset != default_cursor
            || stored_definition_count != stored_default_count
        {
            return None;
        }
        require_scalar(
            table.scalar::<u32>(row, deformation_logical_morph_field::LOGICAL_MESH),
            u32::try_from(row).ok()?,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFINITION_OFFSET),
            definition_cursor,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFINITION_COUNT),
            stored_definition_count,
        )?;
        require_optional_record_reference(
            table,
            row,
            deformation_logical_morph_field::DEFINITIONS,
            stored_definition_count != 0,
            definitions,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFAULT_WEIGHT_OFFSET),
            default_cursor,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_logical_morph_field::DEFAULT_WEIGHT_COUNT),
            stored_default_count,
        )?;
        require_optional_sequence_reference(
            table,
            row,
            deformation_logical_morph_field::DEFAULT_WEIGHTS,
            stored_default_count != 0,
            defaults,
        )?;
        if stored_definition_count != 0 {
            let definitions = definition_values?;
            let definition_width = usize::try_from(stored_definition_count).ok()?;
            maximum_uniqueness_count = maximum_uniqueness_count.max(definition_width);
            let mut identity_marks = match fresh {
                Some(_) => None,
                None => Some(
                    uniqueness_scratch
                        .as_deref_mut()?
                        .begin_group(definition_width)?,
                ),
            };
            let range = checked_range(
                definition_cursor,
                stored_definition_count,
                definitions.rows(),
            )?;
            for (target, definition_row) in range.enumerate() {
                observe_work(1)?;
                require_scalar(
                    definitions.scalar::<u32>(
                        definition_row,
                        deformation_morph_definition_field::LOGICAL_MESH,
                    ),
                    u32::try_from(row).ok()?,
                )?;
                require_scalar(
                    definitions.scalar::<u32>(
                        definition_row,
                        deformation_morph_definition_field::TARGET_ORDINAL,
                    ),
                    u32::try_from(target).ok()?,
                )?;
                let identity = definitions.scalar::<u32>(
                    definition_row,
                    deformation_morph_definition_field::STABLE_IDENTITY,
                )?;
                if identity >= stored_definition_count {
                    return None;
                }
                if let Some(marks) = identity_marks.as_mut() {
                    marks.mark(identity)?;
                }
            }
        }
        if stored_default_count != 0 {
            let stored = default_values?.as_slice();
            let range = checked_range(default_cursor, stored_default_count, stored.len())?;
            let stored = stored.get(range)?;
            observe_work(u64::from(stored_default_count))?;
            if stored.iter().any(|weight| !weight.is_finite()) {
                return None;
            }
            if let Some(logical) = expected_logical.as_ref() {
                for (stored, expected) in stored.iter().zip(logical.authored().default_weights()) {
                    require_float32_bits(Some(*stored), *expected)?;
                }
            }
        }
        if let Some(logical) = expected_logical.as_ref()
            && (logical.definitions().offset() != stored_definition_offset
                || logical.definitions().count() != stored_definition_count
                || logical.target_count() != stored_definition_count
                || logical.default_weight_offset() != stored_default_offset
                || usize::try_from(stored_default_count).ok()?
                    != logical.authored().default_weights().len())
        {
            return None;
        }
        definition_cursor = definition_cursor.checked_add(stored_definition_count)?;
        default_cursor = default_cursor.checked_add(stored_default_count)?;
    }
    (definition_cursor == definition_count && default_cursor == default_weight_count)
        .then_some((Some(table), maximum_uniqueness_count))
}

#[allow(clippy::too_many_arguments)]
fn validate_morph_deltas<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<DeformationMorphDeltaTableSchema>>,
    values: Option<CanonicalSequenceRef<f32, DeformationMorphValueSchema>>,
    sparse_indices: Option<CanonicalSequenceRef<u32, DeformationMorphSparseIndexSchema>>,
    targets: Option<CanonicalRecordRef<DeformationMorphTargetTableSchema>>,
    geometry_primitives: Option<CanonicalRecordView<'storage>>,
    delta_count: u32,
    value_count: u32,
    sparse_index_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<Option<CanonicalRecordView<'storage>>> {
    let Some(reference) = reference else {
        return (delta_count == 0).then_some(None);
    };
    let table = session.canonical_records(reference)?;
    let target_table = session.canonical_records(targets?)?;
    let primitive_table = geometry_primitives?;
    let stored_values = match values {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let stored_indices = match sparse_indices {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let mut target_cursor = 0_u32;
    let mut delta_cursor = 0_u32;
    let mut value_cursor = 0_u32;
    let mut sparse_cursor = 0_u32;
    for primitive_index in 0..primitive_table.rows() {
        observe_work(1)?;
        let expected_primitive = match fresh {
            Some(fresh) => Some(fresh.deformation.primitive(primitive_index)?),
            None => None,
        };
        let primitive_id = u32::try_from(primitive_index).ok()?;
        let vertex_count = primitive_table
            .scalar::<u32>(primitive_index, geometry_primitive_field::VERTEX_COUNT)?;
        let mut target_ordinal = 0_u32;
        while usize::try_from(target_cursor).ok()? < target_table.rows()
            && target_table.scalar::<u32>(
                usize::try_from(target_cursor).ok()?,
                deformation_morph_target_field::PRIMITIVE,
            )? == primitive_id
        {
            observe_work(1)?;
            let target_row = usize::try_from(target_cursor).ok()?;
            require_scalar(
                target_table
                    .scalar::<u32>(target_row, deformation_morph_target_field::MORPH_TARGET),
                target_cursor,
            )?;
            require_scalar(
                target_table
                    .scalar::<u32>(target_row, deformation_morph_target_field::TARGET_ORDINAL),
                target_ordinal,
            )?;
            let stored_delta_offset = target_table
                .scalar::<u32>(target_row, deformation_morph_target_field::DELTA_OFFSET)?;
            let stored_delta_count = target_table
                .scalar::<u32>(target_row, deformation_morph_target_field::DELTA_COUNT)?;
            require_scalar(Some(stored_delta_offset), delta_cursor)?;
            require_optional_record_reference(
                target_table,
                target_row,
                deformation_morph_target_field::DELTAS,
                stored_delta_count != 0,
                Some(reference),
            )?;
            let expected_target = match expected_primitive.as_ref() {
                Some(primitive) => {
                    let target = primitive.morph_target(usize::try_from(target_ordinal).ok()?)?;
                    if target.primitive_index() != primitive_id
                        || target.target_ordinal() != target_ordinal
                        || u32::try_from(target.delta_count()).ok()? != stored_delta_count
                    {
                        return None;
                    }
                    Some(target)
                }
                None => None,
            };
            let mut previous_semantic = None;
            for delta_ordinal in 0..stored_delta_count {
                observe_work(1)?;
                let row = usize::try_from(delta_cursor).ok()?;
                let semantic = table.scalar::<u8>(row, deformation_morph_delta_field::SEMANTIC)?;
                if !matches!(
                    semantic,
                    DEFORMATION_MORPH_POSITION
                        | DEFORMATION_MORPH_NORMAL
                        | DEFORMATION_MORPH_TANGENT
                ) || previous_semantic.is_some_and(|previous| semantic <= previous)
                {
                    return None;
                }
                previous_semantic = Some(semantic);
                let payload_kind =
                    table.scalar::<u8>(row, deformation_morph_delta_field::PAYLOAD_KIND)?;
                if !matches!(
                    payload_kind,
                    DEFORMATION_MORPH_DENSE | DEFORMATION_MORPH_SPARSE
                ) {
                    return None;
                }
                let logical_count =
                    table.scalar::<u32>(row, deformation_morph_delta_field::LOGICAL_COUNT)?;
                let explicit_count =
                    table.scalar::<u32>(row, deformation_morph_delta_field::EXPLICIT_COUNT)?;
                let stored_value_offset =
                    table.scalar::<u32>(row, deformation_morph_delta_field::VALUE_OFFSET)?;
                let stored_value_count =
                    table.scalar::<u32>(row, deformation_morph_delta_field::VALUE_COUNT)?;
                let stored_sparse_offset =
                    table.scalar::<u32>(row, deformation_morph_delta_field::SPARSE_INDEX_OFFSET)?;
                let stored_sparse_count =
                    table.scalar::<u32>(row, deformation_morph_delta_field::SPARSE_INDEX_COUNT)?;
                let expected_value_count = explicit_count.checked_mul(MORPH_VECTOR_WIDTH)?;
                if logical_count != vertex_count
                    || stored_value_offset != value_cursor
                    || stored_value_count != expected_value_count
                    || stored_sparse_offset != sparse_cursor
                    || match payload_kind {
                        DEFORMATION_MORPH_DENSE => {
                            explicit_count != logical_count || stored_sparse_count != 0
                        }
                        DEFORMATION_MORPH_SPARSE => stored_sparse_count != explicit_count,
                        _ => true,
                    }
                {
                    return None;
                }
                require_scalar(
                    table.scalar::<u32>(row, deformation_morph_delta_field::MORPH_DELTA),
                    delta_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_morph_delta_field::MORPH_TARGET),
                    target_cursor,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_morph_delta_field::VALUES,
                    stored_value_count != 0,
                    values,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_morph_delta_field::SPARSE_INDICES,
                    stored_sparse_count != 0,
                    sparse_indices,
                )?;
                let expected_delta = match expected_target.as_ref() {
                    Some(target) => Some(target.delta(usize::try_from(delta_ordinal).ok()?)?),
                    None => None,
                };
                let (expected_payload_kind, authored_values, authored_indices) =
                    match expected_delta
                        .as_ref()
                        .map(|delta| delta.authored_payload())
                    {
                        None => (None, &[][..], &[][..]),
                        Some(MorphDeltaPayloadAuthoring::Dense { values }) => {
                            (Some(DeformationMorphPayloadKind::Dense), values, &[][..])
                        }
                        Some(MorphDeltaPayloadAuthoring::Sparse {
                            indices, values, ..
                        }) => (Some(DeformationMorphPayloadKind::Sparse), values, indices),
                    };
                if let Some(delta) = expected_delta.as_ref()
                    && (delta.semantic() != delta.authored().semantic()
                        || encode_deformation_morph_semantic(delta.semantic()) != semantic
                        || delta.payload_kind() != expected_payload_kind?
                        || encode_deformation_morph_payload(
                            delta.payload_kind() == DeformationMorphPayloadKind::Sparse,
                        ) != payload_kind
                        || delta.logical_count() != logical_count
                        || delta.explicit_count() != explicit_count
                        || delta.value_offset() != value_cursor
                        || delta.value_count() != stored_value_count
                        || delta.sparse_index_offset() != sparse_cursor
                        || delta.sparse_index_count() != stored_sparse_count
                        || u32::try_from(authored_values.len()).ok()? != stored_value_count
                        || u32::try_from(authored_indices.len()).ok()? != stored_sparse_count)
                {
                    return None;
                }
                if stored_value_count != 0 {
                    let stored = stored_values?.as_slice();
                    let range = checked_range(value_cursor, stored_value_count, stored.len())?;
                    let stored = stored.get(range)?;
                    observe_work(u64::from(stored_value_count))?;
                    if stored.iter().any(|value| !value.is_finite()) {
                        return None;
                    }
                    if expected_delta.is_some() {
                        for (stored, expected) in stored.iter().zip(authored_values) {
                            require_float32_bits(Some(*stored), *expected)?;
                        }
                    }
                }
                if stored_sparse_count != 0 {
                    let stored = stored_indices?.as_slice();
                    let range = checked_range(sparse_cursor, stored_sparse_count, stored.len())?;
                    let stored = stored.get(range)?;
                    observe_work(u64::from(stored_sparse_count))?;
                    if stored.iter().any(|index| *index >= logical_count)
                        || stored.windows(2).any(|pair| pair[0] >= pair[1])
                    {
                        return None;
                    }
                    if expected_delta.is_some() && stored != authored_indices {
                        return None;
                    }
                }
                value_cursor = value_cursor.checked_add(stored_value_count)?;
                sparse_cursor = sparse_cursor.checked_add(stored_sparse_count)?;
                delta_cursor = delta_cursor.checked_add(1)?;
            }
            target_cursor = target_cursor.checked_add(1)?;
            target_ordinal = target_ordinal.checked_add(1)?;
        }
        if let Some(primitive) = expected_primitive.as_ref()
            && primitive.morph_target_count() != usize::try_from(target_ordinal).ok()?
        {
            return None;
        }
    }
    (target_cursor == u32::try_from(target_table.rows()).ok()?
        && delta_cursor == delta_count
        && value_cursor == value_count
        && sparse_cursor == sparse_index_count)
        .then_some(Some(target_table))
}

#[allow(clippy::too_many_arguments)]
fn validate_morph_targets<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<DeformationMorphTargetTableSchema>>,
    target_view: Option<CanonicalRecordView<'storage>>,
    deltas: Option<CanonicalRecordRef<DeformationMorphDeltaTableSchema>>,
    primitives: Option<CanonicalRecordRef<DeformationPrimitiveTableSchema>>,
    target_count: u32,
    delta_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<Option<CanonicalRecordView<'storage>>> {
    let Some(reference) = reference else {
        return (target_count == 0).then_some(None);
    };
    let table = match target_view {
        Some(table) => table,
        None => session.canonical_records(reference)?,
    };
    let primitive_table = session.canonical_records(primitives?)?;
    let mut target_cursor = 0_u32;
    let mut delta_cursor = 0_u32;
    for primitive_index in 0..primitive_table.rows() {
        observe_work(1)?;
        let expected_primitive = match fresh {
            Some(fresh) => Some(fresh.deformation.primitive(primitive_index)?),
            None => None,
        };
        let stored_target_offset = primitive_table.scalar::<u32>(
            primitive_index,
            deformation_primitive_field::MORPH_TARGET_OFFSET,
        )?;
        let stored_target_count = primitive_table.scalar::<u32>(
            primitive_index,
            deformation_primitive_field::MORPH_TARGET_COUNT,
        )?;
        require_scalar(Some(stored_target_offset), target_cursor)?;
        require_optional_record_reference(
            primitive_table,
            primitive_index,
            deformation_primitive_field::MORPH_TARGETS,
            stored_target_count != 0,
            Some(reference),
        )?;
        if let Some(primitive) = expected_primitive.as_ref()
            && (primitive.morph_target_count() != usize::try_from(stored_target_count).ok()?)
        {
            return None;
        }
        for target_ordinal in 0..stored_target_count {
            observe_work(1)?;
            let row = usize::try_from(target_cursor).ok()?;
            let primitive_id = u32::try_from(primitive_index).ok()?;
            let stored_delta_offset =
                table.scalar::<u32>(row, deformation_morph_target_field::DELTA_OFFSET)?;
            let stored_delta_count =
                table.scalar::<u32>(row, deformation_morph_target_field::DELTA_COUNT)?;
            require_scalar(
                table.scalar::<u32>(row, deformation_morph_target_field::MORPH_TARGET),
                target_cursor,
            )?;
            require_scalar(
                table.scalar::<u32>(row, deformation_morph_target_field::PRIMITIVE),
                primitive_id,
            )?;
            require_scalar(
                table.scalar::<u32>(row, deformation_morph_target_field::TARGET_ORDINAL),
                target_ordinal,
            )?;
            require_scalar(
                table.scalar::<u32>(row, deformation_morph_target_field::DELTA_OFFSET),
                delta_cursor,
            )?;
            require_scalar(
                table.scalar::<u32>(row, deformation_morph_target_field::DELTA_COUNT),
                stored_delta_count,
            )?;
            require_optional_record_reference(
                table,
                row,
                deformation_morph_target_field::DELTAS,
                stored_delta_count != 0,
                deltas,
            )?;
            if let Some(primitive) = expected_primitive.as_ref() {
                let target = primitive.morph_target(usize::try_from(target_ordinal).ok()?)?;
                if target.primitive_index() != primitive_id
                    || target.target_ordinal() != target_ordinal
                    || u32::try_from(target.delta_count()).ok()? != stored_delta_count
                    || stored_delta_offset != delta_cursor
                {
                    return None;
                }
            }
            delta_cursor = delta_cursor.checked_add(stored_delta_count)?;
            target_cursor = target_cursor.checked_add(1)?;
        }
    }
    (target_cursor == target_count && delta_cursor == delta_count).then_some(Some(primitive_table))
}

#[allow(clippy::too_many_arguments)]
fn validate_primitives<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<DeformationPrimitiveTableSchema>>,
    primitive_view: Option<CanonicalRecordView<'storage>>,
    row_offsets: Option<CanonicalSequenceRef<u32, DeformationInfluenceRowOffsetSchema>>,
    joints: Option<CanonicalSequenceRef<u32, DeformationInfluenceJointSchema>>,
    weights: Option<CanonicalSequenceRef<f32, DeformationInfluenceWeightSchema>>,
    source_orders: Option<CanonicalSequenceRef<u32, DeformationInfluenceSourceOrderSchema>>,
    morph_targets: Option<CanonicalRecordRef<DeformationMorphTargetTableSchema>>,
    geometry_primitives: Option<CanonicalRecordView<'storage>>,
    primitive_count: u32,
    row_offset_count: u32,
    influence_count: u32,
    morph_target_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    mut uniqueness_scratch: Option<&mut DeformationUniquenessScratch<'_>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<DeformationPrimitiveViews<'storage>> {
    let Some(reference) = reference else {
        return (primitive_count == 0).then_some(DeformationPrimitiveViews {
            primitives: None,
            influence_joints: None,
            maximum_uniqueness_count: 0,
        });
    };
    let table = match primitive_view {
        Some(table) => table,
        None => session.canonical_records(reference)?,
    };
    let geometry = geometry_primitives?;
    let stored_row_offsets = match row_offsets {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let stored_joints = match joints {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let stored_weights = match weights {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let stored_source_orders = match source_orders {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let mut row_offset_cursor = 0_u32;
    let mut influence_cursor = 0_u32;
    let mut target_cursor = 0_u32;
    let mut maximum_uniqueness_count = 0_usize;
    for row in 0..table.rows() {
        observe_work(1)?;
        let expected_primitive = match fresh {
            Some(fresh) => Some(fresh.deformation.primitive(row)?),
            None => None,
        };
        require_scalar(
            table.scalar::<u32>(row, deformation_primitive_field::PRIMITIVE),
            u32::try_from(row).ok()?,
        )?;
        let vertex_count = geometry.scalar::<u32>(row, geometry_primitive_field::VERTEX_COUNT)?;
        let influences_present = require_presence(
            table.scalar::<u8>(row, deformation_primitive_field::INFLUENCE_PRESENCE),
        )?;
        let stored_row_offset_offset =
            table.scalar::<u32>(row, deformation_primitive_field::ROW_OFFSET_OFFSET)?;
        let stored_row_offset_count =
            table.scalar::<u32>(row, deformation_primitive_field::ROW_OFFSET_COUNT)?;
        let stored_influence_offset =
            table.scalar::<u32>(row, deformation_primitive_field::INFLUENCE_OFFSET)?;
        let stored_influence_count =
            table.scalar::<u32>(row, deformation_primitive_field::INFLUENCE_COUNT)?;
        match influences_present {
            StoredPresence::Absent => {
                for field in [
                    deformation_primitive_field::ROW_OFFSET_OFFSET,
                    deformation_primitive_field::ROW_OFFSET_COUNT,
                    deformation_primitive_field::INFLUENCE_OFFSET,
                    deformation_primitive_field::INFLUENCE_COUNT,
                ] {
                    require_scalar(table.scalar::<u32>(row, field), 0_u32)?;
                }
                for field in [
                    deformation_primitive_field::ROW_OFFSETS,
                    deformation_primitive_field::INFLUENCE_JOINTS,
                    deformation_primitive_field::INFLUENCE_WEIGHTS,
                    deformation_primitive_field::INFLUENCE_SOURCE_ORDERS,
                ] {
                    require_scalar(table.scalar::<u8>(row, field), 0_u8)?;
                }
            }
            StoredPresence::Present => {
                if stored_row_offset_count != vertex_count.checked_add(1)?
                    || stored_row_offset_offset != row_offset_cursor
                    || stored_influence_offset != influence_cursor
                {
                    return None;
                }
                require_scalar(
                    table.scalar::<u32>(row, deformation_primitive_field::ROW_OFFSET_OFFSET),
                    row_offset_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_primitive_field::ROW_OFFSET_COUNT),
                    stored_row_offset_count,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_primitive_field::ROW_OFFSETS,
                    true,
                    row_offsets,
                )?;
                let stored = stored_row_offsets?.as_slice();
                let range =
                    checked_range(row_offset_cursor, stored_row_offset_count, stored.len())?;
                let stored_offsets = stored.get(range)?;
                observe_work(u64::from(stored_row_offset_count))?;
                if stored_offsets.first().copied() != Some(0)
                    || stored_offsets.last().copied() != Some(stored_influence_count)
                    || stored_offsets
                        .iter()
                        .any(|offset| *offset > stored_influence_count)
                    || stored_offsets.windows(2).any(|pair| pair[0] > pair[1])
                {
                    return None;
                }
                require_scalar(
                    table.scalar::<u32>(row, deformation_primitive_field::INFLUENCE_OFFSET),
                    influence_cursor,
                )?;
                require_scalar(
                    table.scalar::<u32>(row, deformation_primitive_field::INFLUENCE_COUNT),
                    stored_influence_count,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_primitive_field::INFLUENCE_JOINTS,
                    stored_influence_count != 0,
                    joints,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_primitive_field::INFLUENCE_WEIGHTS,
                    stored_influence_count != 0,
                    weights,
                )?;
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_primitive_field::INFLUENCE_SOURCE_ORDERS,
                    stored_influence_count != 0,
                    source_orders,
                )?;
                if stored_influence_count != 0 {
                    let joint_values = stored_joints?.as_slice();
                    let weight_values = stored_weights?.as_slice();
                    let order_values = stored_source_orders?.as_slice();
                    let joint_range = checked_range(
                        influence_cursor,
                        stored_influence_count,
                        joint_values.len(),
                    )?;
                    let weight_range = checked_range(
                        influence_cursor,
                        stored_influence_count,
                        weight_values.len(),
                    )?;
                    let order_range = checked_range(
                        influence_cursor,
                        stored_influence_count,
                        order_values.len(),
                    )?;
                    let joint_values = joint_values.get(joint_range)?;
                    let weight_values = weight_values.get(weight_range)?;
                    let order_values = order_values.get(order_range)?;
                    observe_work(u64::from(stored_influence_count).checked_mul(3)?)?;
                    if weight_values
                        .iter()
                        .any(|weight| !weight.is_finite() || *weight < 0.0)
                    {
                        return None;
                    }
                    for offsets in stored_offsets.windows(2) {
                        let start = usize::try_from(offsets[0]).ok()?;
                        let end = usize::try_from(offsets[1]).ok()?;
                        let orders = order_values.get(start..end)?;
                        let width = orders.len();
                        maximum_uniqueness_count = maximum_uniqueness_count.max(width);
                        let mut source_order_marks = match fresh {
                            Some(_) => None,
                            None => Some(uniqueness_scratch.as_deref_mut()?.begin_group(width)?),
                        };
                        for order in orders.iter().copied() {
                            observe_work(1)?;
                            match source_order_marks.as_mut() {
                                Some(marks) => marks.mark(order)?,
                                None => {
                                    let order = usize::try_from(order).ok()?;
                                    (order < width).then_some(())?;
                                }
                            }
                        }
                    }
                    if let Some(primitive) = expected_primitive.as_ref() {
                        let Presence::Present(influences) = primitive.authored().influences()
                        else {
                            return None;
                        };
                        if stored_offsets != influences.vertex_offsets()
                            || joint_values.len() != influences.influences().len()
                        {
                            return None;
                        }
                        for (((joint, weight), source_order), authored) in joint_values
                            .iter()
                            .zip(weight_values)
                            .zip(order_values)
                            .zip(influences.influences())
                        {
                            if *joint != authored.joint()
                                || *source_order != authored.source_order()
                            {
                                return None;
                            }
                            require_float32_bits(Some(*weight), authored.weight())?;
                        }
                    }
                }
                row_offset_cursor = row_offset_cursor.checked_add(stored_row_offset_count)?;
                influence_cursor = influence_cursor.checked_add(stored_influence_count)?;
            }
        }
        if let Some(primitive) = expected_primitive.as_ref()
            && (primitive.influence_row_offset_offset() != stored_row_offset_offset
                || primitive.influence_row_offset_count() != stored_row_offset_count
                || primitive.influence_offset() != stored_influence_offset
                || primitive.influence_count() != stored_influence_count
                || matches!(primitive.authored().influences(), Presence::Present(_))
                    != (influences_present == StoredPresence::Present))
        {
            return None;
        }
        let stored_target_offset =
            table.scalar::<u32>(row, deformation_primitive_field::MORPH_TARGET_OFFSET)?;
        let stored_target_count =
            table.scalar::<u32>(row, deformation_primitive_field::MORPH_TARGET_COUNT)?;
        require_scalar(Some(stored_target_offset), target_cursor)?;
        require_scalar(
            table.scalar::<u32>(row, deformation_primitive_field::MORPH_TARGET_OFFSET),
            target_cursor,
        )?;
        require_scalar(
            table.scalar::<u32>(row, deformation_primitive_field::MORPH_TARGET_COUNT),
            stored_target_count,
        )?;
        require_optional_record_reference(
            table,
            row,
            deformation_primitive_field::MORPH_TARGETS,
            stored_target_count != 0,
            morph_targets,
        )?;
        if let Some(primitive) = expected_primitive.as_ref()
            && primitive.morph_target_count() != usize::try_from(stored_target_count).ok()?
        {
            return None;
        }
        target_cursor = target_cursor.checked_add(stored_target_count)?;
    }
    (row_offset_cursor == row_offset_count
        && influence_cursor == influence_count
        && target_cursor == morph_target_count)
        .then_some(DeformationPrimitiveViews {
            primitives: Some(table),
            influence_joints: stored_joints,
            maximum_uniqueness_count,
        })
}

#[allow(clippy::too_many_arguments)]
fn validate_mesh_instances<'storage, R, NR>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<DeformationMeshInstanceTableSchema>>,
    skins: Option<CanonicalRecordRef<DeformationSkinTableSchema>>,
    morph_weights: Option<CanonicalSequenceRef<f32, DeformationInstanceMorphWeightSchema>>,
    logical_morphs: Option<CanonicalRecordView<'storage>>,
    geometry_mesh_instances: Option<CanonicalRecordView<'storage>>,
    skin_count: u32,
    mesh_instance_count: u32,
    morph_weight_count: u32,
    fresh: Option<&FreshDeformationEquivalence<'_, '_, '_, R, NR>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<Option<CanonicalRecordView<'storage>>> {
    let Some(reference) = reference else {
        return (mesh_instance_count == 0).then_some(None);
    };
    let table = session.canonical_records(reference)?;
    let logical_morphs = logical_morphs?;
    let geometry_mesh_instances = geometry_mesh_instances?;
    let stored_weights = match morph_weights {
        Some(reference) => Some(session.canonical_sequence(reference)?),
        None => None,
    };
    let mut weight_cursor = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        let expected_instance = match fresh {
            Some(fresh) => Some(fresh.deformation.mesh_instance(row)?),
            None => None,
        };
        require_scalar(
            table.scalar::<u32>(row, deformation_mesh_instance_field::MESH_INSTANCE),
            u32::try_from(row).ok()?,
        )?;
        let logical_mesh = geometry_mesh_instances
            .scalar::<u32>(row, geometry_mesh_instance_field::LOGICAL_MESH)?;
        let logical_row = usize::try_from(logical_mesh).ok()?;
        if logical_row >= logical_morphs.rows() {
            return None;
        }
        let target_count = logical_morphs.scalar::<u32>(
            logical_row,
            deformation_logical_morph_field::DEFINITION_COUNT,
        )?;
        let skin_present = require_presence(
            table.scalar::<u8>(row, deformation_mesh_instance_field::SKIN_PRESENCE),
        )?;
        let stored_skin = table.scalar::<u32>(row, deformation_mesh_instance_field::SKIN)?;
        match skin_present {
            StoredPresence::Absent => {
                require_scalar(Some(stored_skin), 0_u32)?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_mesh_instance_field::SKINS),
                    0_u8,
                )?;
            }
            StoredPresence::Present => {
                if stored_skin >= skin_count {
                    return None;
                }
                require_optional_record_reference(
                    table,
                    row,
                    deformation_mesh_instance_field::SKINS,
                    true,
                    skins,
                )?;
            }
        }
        if let Some(instance) = expected_instance.as_ref() {
            match (instance.authored().skin(), skin_present) {
                (Presence::Absent, StoredPresence::Absent) if instance.skin().is_none() => {}
                (Presence::Present(skin), StoredPresence::Present)
                    if instance.skin() == Some(skin) && stored_skin == skin.index() => {}
                _ => return None,
            }
        }
        let weights_present = require_presence(
            table.scalar::<u8>(row, deformation_mesh_instance_field::MORPH_WEIGHTS_PRESENCE),
        )?;
        let stored_weight_offset =
            table.scalar::<u32>(row, deformation_mesh_instance_field::MORPH_WEIGHT_OFFSET)?;
        let stored_weight_count =
            table.scalar::<u32>(row, deformation_mesh_instance_field::MORPH_WEIGHT_COUNT)?;
        match weights_present {
            StoredPresence::Absent => {
                require_scalar(
                    Some(stored_weight_offset.checked_add(stored_weight_count)?),
                    0_u32,
                )?;
                require_scalar(
                    table.scalar::<u8>(row, deformation_mesh_instance_field::MORPH_WEIGHTS),
                    0_u8,
                )?;
            }
            StoredPresence::Present => {
                if stored_weight_offset != weight_cursor || stored_weight_count != target_count {
                    return None;
                }
                require_optional_sequence_reference(
                    table,
                    row,
                    deformation_mesh_instance_field::MORPH_WEIGHTS,
                    stored_weight_count != 0,
                    morph_weights,
                )?;
                if stored_weight_count != 0 {
                    let stored = stored_weights?.as_slice();
                    let range = checked_range(weight_cursor, stored_weight_count, stored.len())?;
                    let stored = stored.get(range)?;
                    observe_work(u64::from(stored_weight_count))?;
                    if stored.iter().any(|weight| !weight.is_finite()) {
                        return None;
                    }
                    if let Some(instance) = expected_instance.as_ref() {
                        let Presence::Present(weights) = instance.authored().morph_weights() else {
                            return None;
                        };
                        for (stored, expected) in stored.iter().zip(weights) {
                            require_float32_bits(Some(*stored), *expected)?;
                        }
                    }
                }
                weight_cursor = weight_cursor.checked_add(stored_weight_count)?;
            }
        }
        if let Some(instance) = expected_instance.as_ref()
            && (instance.morph_weight_offset() != stored_weight_offset
                || instance.morph_weight_count() != stored_weight_count
                || matches!(instance.authored().morph_weights(), Presence::Present(_))
                    != (weights_present == StoredPresence::Present))
        {
            return None;
        }
    }
    (weight_cursor == morph_weight_count).then_some(Some(table))
}

#[allow(clippy::too_many_arguments)]
fn validate_canonical_cross_domain_semantics(
    logical_meshes: Option<CanonicalRecordView<'_>>,
    geometry_mesh_instances: Option<CanonicalRecordView<'_>>,
    logical_morphs: Option<CanonicalRecordView<'_>>,
    primitives: Option<CanonicalRecordView<'_>>,
    mesh_instances: Option<CanonicalRecordView<'_>>,
    skins: Option<CanonicalRecordView<'_>>,
    influence_joints: Option<SequenceView<'_, u32>>,
    palette_scratch: &mut [Option<u32>],
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(logical_meshes) = logical_meshes else {
        return (logical_morphs.is_none()
            && primitives.is_none()
            && geometry_mesh_instances.is_none()
            && mesh_instances.is_none()
            && influence_joints.is_none())
        .then_some(());
    };
    let logical_morphs = logical_morphs?;
    if logical_meshes.rows() != logical_morphs.rows()
        || palette_scratch.len() < logical_meshes.rows()
    {
        return None;
    }
    let instance_views = match (geometry_mesh_instances, mesh_instances) {
        (None, None) => None,
        (Some(geometry), Some(deformation)) if geometry.rows() == deformation.rows() => {
            Some((geometry, deformation))
        }
        _ => return None,
    };
    let primitive_rows = primitives.map_or(0, |table| table.rows());
    let joint_values = influence_joints.map(|joints| joints.as_slice());
    let mut primitive_cursor = 0_u32;
    for logical_mesh_row in 0..logical_meshes.rows() {
        observe_work(1)?;
        let palette_mark = palette_scratch.get_mut(logical_mesh_row)?;
        *palette_mark = None;
        require_scalar(
            logical_meshes
                .scalar::<u32>(logical_mesh_row, geometry_logical_mesh_field::LOGICAL_MESH),
            u32::try_from(logical_mesh_row).ok()?,
        )?;
        let primitive_offset = logical_meshes.scalar::<u32>(
            logical_mesh_row,
            geometry_logical_mesh_field::PRIMITIVE_OFFSET,
        )?;
        let primitive_count = logical_meshes.scalar::<u32>(
            logical_mesh_row,
            geometry_logical_mesh_field::PRIMITIVE_COUNT,
        )?;
        require_scalar(Some(primitive_offset), primitive_cursor)?;
        let target_width = logical_morphs.scalar::<u32>(
            logical_mesh_row,
            deformation_logical_morph_field::DEFINITION_COUNT,
        )?;
        let primitive_range = checked_range(primitive_offset, primitive_count, primitive_rows)?;
        for primitive_row in primitive_range {
            observe_work(1)?;
            let primitives = primitives?;
            require_scalar(
                primitives.scalar::<u32>(
                    primitive_row,
                    deformation_primitive_field::MORPH_TARGET_COUNT,
                ),
                target_width,
            )?;
            let influence_offset = primitives
                .scalar::<u32>(primitive_row, deformation_primitive_field::INFLUENCE_OFFSET)?;
            let influence_count = primitives
                .scalar::<u32>(primitive_row, deformation_primitive_field::INFLUENCE_COUNT)?;
            if influence_count == 0 {
                continue;
            }
            let joint_values = joint_values?;
            let influence_range =
                checked_range(influence_offset, influence_count, joint_values.len())?;
            observe_work(u64::from(influence_count))?;
            for joint in joint_values.get(influence_range)?.iter().copied() {
                record_palette_joint(palette_mark, joint);
            }
        }
        primitive_cursor = primitive_offset.checked_add(primitive_count)?;
    }
    if usize::try_from(primitive_cursor).ok()? != primitive_rows {
        return None;
    }
    let Some((geometry_instances, deformation_instances)) = instance_views else {
        return Some(());
    };
    for instance_row in 0..deformation_instances.rows() {
        observe_work(1)?;
        let logical_mesh = usize::try_from(
            geometry_instances
                .scalar::<u32>(instance_row, geometry_mesh_instance_field::LOGICAL_MESH)?,
        )
        .ok()?;
        let skin_present = require_presence(
            deformation_instances
                .scalar::<u8>(instance_row, deformation_mesh_instance_field::SKIN_PRESENCE),
        )?;
        if skin_present == StoredPresence::Absent {
            continue;
        }
        let palette_maximum = *palette_scratch.get(logical_mesh)?;
        let skin = usize::try_from(
            deformation_instances
                .scalar::<u32>(instance_row, deformation_mesh_instance_field::SKIN)?,
        )
        .ok()?;
        let joint_count = skins?.scalar::<u32>(skin, deformation_skin_field::JOINT_COUNT)?;
        if !palette_fits_skin(palette_maximum, skin_present, joint_count) {
            return None;
        }
    }
    Some(())
}

fn require_optional_record_reference<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    present: bool,
    reference: Option<CanonicalRecordRef<Tag>>,
) -> Option<()>
where
    Tag: Copy + Send + Sync + 'static,
{
    if present {
        require_scalar(table.record_ref::<Tag>(row, field), reference?)
    } else {
        require_scalar(table.scalar::<u8>(row, field), 0_u8)
    }
}

fn require_optional_sequence_reference<T, Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    present: bool,
    reference: Option<CanonicalSequenceRef<T, Tag>>,
) -> Option<()>
where
    T: StoredScalar,
    Tag: Copy + Send + Sync + 'static,
{
    if present {
        require_scalar(table.sequence_ref::<T, Tag>(row, field), reference?)
    } else {
        require_scalar(table.scalar::<u8>(row, field), 0_u8)
    }
}

fn require_optional_utf8_reference<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    present: bool,
    reference: Option<CanonicalUtf8Ref<Tag>>,
) -> Option<()>
where
    Tag: Copy + Send + Sync + 'static,
{
    if present {
        require_scalar(table.utf8_ref::<Tag>(row, field), reference?)
    } else {
        require_scalar(table.scalar::<u8>(row, field), 0_u8)
    }
}

fn require_presence(actual: Option<u8>) -> Option<StoredPresence> {
    match actual? {
        PRESENCE_ABSENT => Some(StoredPresence::Absent),
        PRESENCE_PRESENT => Some(StoredPresence::Present),
        _ => None,
    }
}

fn is_authored_node_index(index: u32, node_rows: usize) -> bool {
    index != 0 && usize::try_from(index).is_ok_and(|index| index < node_rows)
}

fn checked_utf8_range(value: &str, offset: u32, count: u32) -> Option<&str> {
    value.get(checked_range(offset, count, value.len())?)
}

fn validate_entity_name_syntax(value: &str) -> Option<()> {
    (value.len() <= MAX_ENTITY_NAME_BYTES).then_some(())?;
    (!value.chars().any(|character| character.is_control())).then_some(())
}

fn record_palette_joint(maximum: &mut Option<u32>, joint: u32) {
    *maximum = Some((*maximum).map_or(joint, |maximum| maximum.max(joint)));
}

fn palette_fits_skin(maximum: Option<u32>, skin_present: StoredPresence, joint_count: u32) -> bool {
    skin_present == StoredPresence::Absent || maximum.is_none_or(|maximum| maximum < joint_count)
}

fn validate_affine_matrices(values: &[f32]) -> Option<()> {
    let mut matrices = values.chunks_exact(usize::try_from(MATRIX_SCALAR_COUNT).ok()?);
    for matrix in &mut matrices {
        if matrix.iter().any(|value| !value.is_finite())
            || matrix.get(3).copied()? != 0.0
            || matrix.get(7).copied()? != 0.0
            || matrix.get(11).copied()? != 0.0
            || matrix.get(15).copied()? != 1.0
        {
            return None;
        }
    }
    matrices.remainder().is_empty().then_some(())
}

fn checked_range(offset: u32, count: u32, len: usize) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(offset).ok()?;
    let count = usize::try_from(count).ok()?;
    let end = start.checked_add(count)?;
    (end <= len).then_some(start..end)
}

fn require_float32_bits(actual: Option<f32>, expected: f32) -> Option<()> {
    (actual?.to_bits() == expected.to_bits()).then_some(())
}

#[cfg(test)]
mod tests {
    use super::{
        DeformationUniquenessScratch, StoredPresence, checked_utf8_range, is_authored_node_index,
        palette_fits_skin, record_palette_joint, validate_entity_name_syntax,
    };
    use crate::entity::{EntityName, MAX_ENTITY_NAME_BYTES};

    #[test]
    fn authored_node_indices_exclude_the_synthetic_root() {
        assert!(!is_authored_node_index(0, 3));
        assert!(is_authored_node_index(1, 3));
        assert!(is_authored_node_index(2, 3));
        assert!(!is_authored_node_index(3, 3));
        assert!(!is_authored_node_index(u32::MAX, 3));
    }

    #[test]
    fn utf8_ranges_require_code_point_boundaries() {
        let value = "a\u{e9}z";

        assert_eq!(checked_utf8_range(value, 1, 2), Some("\u{e9}"));
        assert_eq!(checked_utf8_range(value, 2, 0), None);
        assert_eq!(checked_utf8_range(value, 1, 1), None);
    }

    #[test]
    fn entity_name_syntax_is_checked_without_ownership() {
        for value in ["", "valid \u{540d}\u{79f0}", "bad\nname"] {
            assert_eq!(
                validate_entity_name_syntax(value).is_some(),
                EntityName::try_new(value).is_ok()
            );
        }

        let oversized = [b'x'; MAX_ENTITY_NAME_BYTES + 1];
        let oversized = std::str::from_utf8(&oversized).expect("ASCII fixture");
        assert_eq!(validate_entity_name_syntax(oversized), None);
        assert!(EntityName::try_new(oversized).is_err());
    }

    #[test]
    fn palette_summary_covers_the_full_u32_domain() {
        let mut maximum = None;
        record_palette_joint(&mut maximum, 0);
        assert_eq!(maximum, Some(0));
        record_palette_joint(&mut maximum, u32::MAX);
        assert_eq!(maximum, Some(u32::MAX));

        assert!(palette_fits_skin(maximum, StoredPresence::Absent, 0));
        assert!(!palette_fits_skin(
            maximum,
            StoredPresence::Present,
            u32::MAX
        ));
        assert!(palette_fits_skin(None, StoredPresence::Present, 0));
        assert!(palette_fits_skin(Some(0), StoredPresence::Present, 1));
        assert!(!palette_fits_skin(Some(0), StoredPresence::Present, 0));
    }

    #[test]
    fn uniqueness_groups_reuse_marks_without_prefix_rescans() {
        let mut storage = [0_u64; 3];
        let mut scratch = DeformationUniquenessScratch::new(&mut storage);

        {
            let mut first = scratch.begin_group(3).expect("first group fits");
            assert_eq!(first.mark(2), Some(()));
            assert_eq!(first.mark(0), Some(()));
            assert_eq!(first.mark(1), Some(()));
            assert_eq!(first.mark(1), None);
        }

        let mut second = scratch.begin_group(2).expect("second group fits");
        assert_eq!(second.mark(1), Some(()));
        assert_eq!(second.mark(0), Some(()));
        assert_eq!(second.mark(2), None);
    }
}
