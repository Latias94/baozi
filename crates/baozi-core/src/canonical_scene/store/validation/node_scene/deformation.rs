use baozi_operation::__retained_store_integration::activation::{
    CanonicalValidationMember, CanonicalValidationSession,
};
use baozi_operation::{
    OperationOwner, ProgressPhase, ProgressUnit, RecordBlock, ScalarSequence, StoreWorkError,
    Utf8Block,
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
};
use crate::canonical_scene::store::schema::{CanonicalBlockRef, PRESENCE_ABSENT, PRESENCE_PRESENT};
use crate::canonical_scene::store::typed_storage::{
    NodeStorage, StoredLogicalMesh, StoredMeshInstance, StoredPrimitive,
};
use crate::entity::MAX_ENTITY_NAME_BYTES;

const MATRIX_SCALAR_COUNT: u32 = 16;
const MORPH_VECTOR_WIDTH: u32 = 3;

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
pub(super) struct DeformationGeometryDependencies<'projection> {
    pub(super) logical_meshes: &'projection [StoredLogicalMesh],
    pub(super) primitives: &'projection [StoredPrimitive],
    pub(super) mesh_instances: &'projection [StoredMeshInstance],
}

#[derive(Clone, Copy)]
pub(super) struct DeformationValidationDependenciesMetered<'projection> {
    pub(super) nodes: &'projection NodeStorage,
    pub(super) node_count: u32,
    pub(super) geometry: DeformationGeometryDependencies<'projection>,
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

/// Projection capacities and immutable payload lengths derived from the sealed Scene plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct DeformationValidationScratchRequirements {
    pub(in crate::canonical_scene::store) skins: usize,
    pub(in crate::canonical_scene::store) morph_definitions: usize,
    pub(in crate::canonical_scene::store) logical_morphs: usize,
    pub(in crate::canonical_scene::store) morph_deltas: usize,
    pub(in crate::canonical_scene::store) morph_targets: usize,
    pub(in crate::canonical_scene::store) primitives: usize,
    pub(in crate::canonical_scene::store) mesh_instances: usize,
    pub(in crate::canonical_scene::store) influence_row_offset_count: usize,
    pub(in crate::canonical_scene::store) influence_count: usize,
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
            influence_row_offset_count: usize::try_from(influence_row_offset_count).ok()?,
            influence_count: usize::try_from(influence_count).ok()?,
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
    nodes: &NodeStorage,
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
            let nodes_match = row.scalar(deformation_skin_field::NODE_COUNT, |value: u32| {
                value as usize == nodes.nodes.len()
            })? == Some(true);
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
    geometry: &[StoredPrimitive],
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
    geometry: &[StoredPrimitive],
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
    geometry: &[StoredMeshInstance],
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
            let Some(logical_mesh) = geometry.get(row_index).map(|shape| shape.logical_mesh.0)
            else {
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
) -> Result<Option<&'storage [u32]>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Ok(expected_len) = usize::try_from(count) else {
        return Ok(None);
    };
    if expected_len == 0 {
        return Ok(member.is_none().then_some(&[]));
    }
    let Some(member) = member else {
        return Ok(None);
    };
    let mut primitive_index = 0_usize;
    let mut previous = 0_u32;
    session.validate_canonical_sequence(member, expected_len, |index, value| {
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

struct DeformationValidatedInfluencePayloads<'storage> {
    row_offsets: &'storage [u32],
    joints: &'storage [u32],
    source_orders: &'storage [u32],
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
) -> Result<Option<DeformationValidatedInfluencePayloads<'storage>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Ok(node_rows) = usize::try_from(node_count) else {
        return Ok(None);
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
        return Ok(None);
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
    {
        return Ok(None);
    }
    let Some(row_offsets) = validate_influence_row_offsets_metered(
        session,
        members.influence_row_offsets,
        counts.influence_row_offset_count,
        primitives,
    )?
    else {
        return Ok(None);
    };

    let Ok(influence_len) = usize::try_from(counts.influence_count) else {
        return Ok(None);
    };
    let joints = match members.influence_joints {
        None => (influence_len == 0).then_some(&[][..]),
        Some(member) if influence_len != 0 => {
            session.validate_canonical_sequence(member, influence_len, |_index, _value| true)?
        }
        Some(_) => None,
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
    let source_orders = match members.influence_source_orders {
        None => (influence_len == 0).then_some(&[][..]),
        Some(member) if influence_len != 0 => {
            session.validate_canonical_sequence(member, influence_len, |_index, _value| true)?
        }
        Some(_) => None,
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

    let (Some(joints), Some(source_orders)) = (joints, source_orders) else {
        return Ok(None);
    };
    if !influence_weights_valid
        || !instance_weights_valid
        || logical_morphs.iter().try_fold(0_u32, |cursor, projection| {
            if projection.default_weight_offset == cursor {
                cursor.checked_add(projection.default_weight_count)
            } else {
                None
            }
        }) != Some(counts.morph_default_count)
    {
        return Ok(None);
    }
    Ok(Some(DeformationValidatedInfluencePayloads {
        row_offsets,
        joints,
        source_orders,
    }))
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn validate_deformation_algorithms_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    counts: DeformationCanonicalCounts,
    geometry: DeformationGeometryDependencies<'_>,
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
            geometry_logical_mesh.primitives.start() as u32 == geometry_primitive_cursor
                && (geometry_logical_mesh.primitives.start() as u32)
                    .checked_add(geometry_logical_mesh.primitives.len() as u32)
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
        geometry_primitive_cursor = match geometry_primitive_cursor
            .checked_add(geometry_logical_mesh.primitives.len() as u32)
        {
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
            geometry_logical_mesh.primitives.start() as u32,
            geometry_logical_mesh.primitives.len() as u32,
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
                .and_then(|projection| usize::try_from(projection.logical_mesh.0).ok())
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
    dependencies: DeformationValidationDependenciesMetered<'_>,
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
            dependencies.geometry.logical_meshes.len(),
        ),
        (
            counts.primitive_count,
            dependencies.geometry.primitives.len(),
        ),
        (
            counts.mesh_instance_count,
            dependencies.geometry.mesh_instances.len(),
        ),
    ]
    .into_iter()
    .all(|(count, actual)| count as usize == actual);
    if !geometry_presence_valid
        || dependencies.node_count == 0
        || dependencies.node_count as usize != dependencies.nodes.nodes.len()
    {
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
    let Some(influences) = validate_deformation_payloads_metered(
        session,
        members,
        counts,
        dependencies.node_count,
        skins,
        morph_definitions,
        logical_morphs,
        morph_deltas,
        primitives,
    )?
    else {
        return Ok(None);
    };
    let Some(algorithm_maximum) = validate_deformation_algorithms_metered(
        session,
        counts,
        dependencies.geometry,
        skins,
        logical_morphs,
        morph_targets,
        primitives,
        mesh_instances,
        influences.row_offsets,
        influences.joints,
        influences.source_orders,
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

fn checked_range(offset: u32, count: u32, len: usize) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(offset).ok()?;
    let count = usize::try_from(count).ok()?;
    let end = start.checked_add(count)?;
    (end <= len).then_some(start..end)
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
