use baozi_operation::__retained_store_integration::activation::{
    CanonicalLane, CanonicalRecordBuildRow,
};
use baozi_operation::{
    OperationOwner, ProgressPhase, ProgressUnit, RecordBlock, RecordPlan, ScalarSequence,
    SequencePlan, StoreBuildError, StoreControlledError, StorePlanError, StoreRuntimeError,
    StoredScalar, Utf8Block, Utf8Plan,
};

use crate::canonical_scene::store::production::{
    DeformationMorphPayloadKind, MorphDeltaPayloadAuthoring, Presence, ValidatedDeformationBatch,
    ValidatedNodeBatch,
};
use crate::canonical_scene::store::schema::deformation::{
    DEFORMATION_DIRECTORY_FIELD_COUNT, DEFORMATION_LOGICAL_MORPH_FIELD_COUNT,
    DEFORMATION_MESH_INSTANCE_FIELD_COUNT, DEFORMATION_MORPH_DEFINITION_FIELD_COUNT,
    DEFORMATION_MORPH_DELTA_FIELD_COUNT, DEFORMATION_MORPH_TARGET_FIELD_COUNT,
    DEFORMATION_PRIMITIVE_FIELD_COUNT, DEFORMATION_SCHEMA_REVISION, DEFORMATION_SKIN_FIELD_COUNT,
    DeformationDirectorySchema, DeformationInfluenceJointSchema,
    DeformationInfluenceRowOffsetSchema, DeformationInfluenceSourceOrderSchema,
    DeformationInfluenceWeightSchema, DeformationInstanceMorphWeightSchema,
    DeformationInverseBindMatrixSchema, DeformationLogicalMorphTableSchema,
    DeformationMeshInstanceTableSchema, DeformationMorphDefaultWeightSchema,
    DeformationMorphDefinitionTableSchema, DeformationMorphDeltaTableSchema,
    DeformationMorphNameSchema, DeformationMorphSparseIndexSchema,
    DeformationMorphTargetTableSchema, DeformationMorphValueSchema,
    DeformationPrimitiveTableSchema, DeformationSkinCorrectionMatrixSchema,
    DeformationSkinJointNodeSchema, DeformationSkinNameSchema, DeformationSkinTableSchema,
    encode_deformation_morph_payload, encode_deformation_morph_semantic,
    encode_deformation_skin_matrix_spaces, encode_deformation_skin_space,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, PRESENCE_ABSENT, PRESENCE_PRESENT,
};
use crate::canonical_scene::store::typed_storage::{GeometryStorage, NodeStorage};

use super::{
    checked_validation_work_sum, optional_record_validation_work,
    optional_sequence_validation_work, optional_utf8_validation_work, record_validation_work,
};

const MATRIX_SCALAR_COUNT: u32 = 16;

pub(super) struct DeformationMaterializationPlans {
    skins: Option<RecordPlan>,
    logical_morphs: Option<RecordPlan>,
    morph_definitions: Option<RecordPlan>,
    primitives: Option<RecordPlan>,
    morph_targets: Option<RecordPlan>,
    morph_deltas: Option<RecordPlan>,
    mesh_instances: Option<RecordPlan>,
    joint_nodes: Option<SequencePlan<u32>>,
    inverse_bind_matrices: Option<SequencePlan<f32>>,
    skin_correction_matrices: Option<SequencePlan<f32>>,
    skin_names: Option<Utf8Plan>,
    influence_row_offsets: Option<SequencePlan<u32>>,
    influence_joints: Option<SequencePlan<u32>>,
    influence_weights: Option<SequencePlan<f32>>,
    influence_source_orders: Option<SequencePlan<u32>>,
    morph_defaults: Option<SequencePlan<f32>>,
    morph_names: Option<Utf8Plan>,
    morph_values: Option<SequencePlan<f32>>,
    morph_sparse_indices: Option<SequencePlan<u32>>,
    instance_morph_weights: Option<SequencePlan<f32>>,
    directory: RecordPlan,
    skin_count: u32,
    logical_morph_count: u32,
    morph_definition_count: u32,
    primitive_count: u32,
    morph_target_count: u32,
    morph_delta_count: u32,
    mesh_instance_count: u32,
    joint_node_count: u32,
    inverse_bind_scalar_count: u32,
    skin_correction_scalar_count: u32,
    skin_name_byte_count: u32,
    influence_row_offset_count: u32,
    influence_count: u32,
    morph_default_weight_count: u32,
    morph_name_byte_count: u32,
    morph_value_count: u32,
    morph_sparse_index_count: u32,
    instance_morph_weight_count: u32,
    table_block_count: usize,
    payload_block_count: u32,
    canonical_block_count: usize,
}

impl DeformationMaterializationPlans {
    pub(super) fn new<R>(batch: &ValidatedDeformationBatch<'_, R>) -> Result<Self, StorePlanError> {
        let skin_count = batch.skin_count();
        let logical_morph_count = batch.logical_morph_count();
        let morph_definition_count = batch.morph_definition_count();
        let primitive_count = batch.primitive_count();
        let morph_target_count = batch.morph_target_count();
        let morph_delta_count = batch.morph_delta_count();
        let mesh_instance_count = batch.mesh_instance_count();
        let joint_node_count = batch.joint_node_count();
        let inverse_bind_scalar_count = batch.inverse_bind_scalar_count();
        let skin_correction_scalar_count = batch.skin_correction_scalar_count();
        let skin_name_byte_count = batch.skin_name_byte_count();
        let influence_row_offset_count = batch.influence_row_offset_count();
        let influence_count = batch.influence_count();
        let morph_default_weight_count = batch.morph_default_weight_count();
        let morph_name_byte_count = batch.morph_name_byte_count();
        let morph_value_count = batch.morph_value_count();
        let morph_sparse_index_count = batch.morph_sparse_index_count();
        let instance_morph_weight_count = batch.instance_morph_weight_count();

        let skins = record_plan_if_nonempty(skin_count, DEFORMATION_SKIN_FIELD_COUNT)?;
        let logical_morphs =
            record_plan_if_nonempty(logical_morph_count, DEFORMATION_LOGICAL_MORPH_FIELD_COUNT)?;
        let morph_definitions = record_plan_if_nonempty(
            morph_definition_count,
            DEFORMATION_MORPH_DEFINITION_FIELD_COUNT,
        )?;
        let primitives =
            record_plan_if_nonempty(primitive_count, DEFORMATION_PRIMITIVE_FIELD_COUNT)?;
        let morph_targets =
            record_plan_if_nonempty(morph_target_count, DEFORMATION_MORPH_TARGET_FIELD_COUNT)?;
        let morph_deltas =
            record_plan_if_nonempty(morph_delta_count, DEFORMATION_MORPH_DELTA_FIELD_COUNT)?;
        let mesh_instances =
            record_plan_if_nonempty(mesh_instance_count, DEFORMATION_MESH_INSTANCE_FIELD_COUNT)?;
        let joint_nodes = sequence_plan_if_nonempty(joint_node_count)?;
        let inverse_bind_matrices = sequence_plan_if_nonempty(inverse_bind_scalar_count)?;
        let skin_correction_matrices = sequence_plan_if_nonempty(skin_correction_scalar_count)?;
        let skin_names = utf8_plan_if_nonempty(skin_name_byte_count)?;
        let influence_row_offsets = sequence_plan_if_nonempty(influence_row_offset_count)?;
        let influence_joints = sequence_plan_if_nonempty(influence_count)?;
        let influence_weights = sequence_plan_if_nonempty(influence_count)?;
        let influence_source_orders = sequence_plan_if_nonempty(influence_count)?;
        let morph_defaults = sequence_plan_if_nonempty(morph_default_weight_count)?;
        let morph_names = utf8_plan_if_nonempty(morph_name_byte_count)?;
        let morph_values = sequence_plan_if_nonempty(morph_value_count)?;
        let morph_sparse_indices = sequence_plan_if_nonempty(morph_sparse_index_count)?;
        let instance_morph_weights = sequence_plan_if_nonempty(instance_morph_weight_count)?;

        let table_block_count = [
            skins.is_some(),
            logical_morphs.is_some(),
            morph_definitions.is_some(),
            primitives.is_some(),
            morph_targets.is_some(),
            morph_deltas.is_some(),
            mesh_instances.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
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
        })
        .ok_or(StorePlanError::SizeOverflow)?;
        if payload_block_count != batch.payload_block_count() {
            return Err(StorePlanError::InvalidBlockTable);
        }
        let canonical_block_count = 1_usize
            .checked_add(table_block_count)
            .and_then(|count| {
                usize::try_from(payload_block_count)
                    .ok()
                    .and_then(|payloads| count.checked_add(payloads))
            })
            .ok_or(StorePlanError::SizeOverflow)?;

        Ok(Self {
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
            directory: RecordPlan::new(1, DEFORMATION_DIRECTORY_FIELD_COUNT)?,
            skin_count,
            logical_morph_count,
            morph_definition_count,
            primitive_count,
            morph_target_count,
            morph_delta_count,
            mesh_instance_count,
            joint_node_count,
            inverse_bind_scalar_count,
            skin_correction_scalar_count,
            skin_name_byte_count,
            influence_row_offset_count,
            influence_count,
            morph_default_weight_count,
            morph_name_byte_count,
            morph_value_count,
            morph_sparse_index_count,
            instance_morph_weight_count,
            table_block_count,
            payload_block_count,
            canonical_block_count,
        })
    }

    pub(super) const fn canonical_block_count(&self) -> usize {
        self.canonical_block_count
    }

    pub(super) const fn table_block_count(&self) -> usize {
        self.table_block_count
    }

    pub(super) const fn payload_block_count(&self) -> u32 {
        self.payload_block_count
    }

    pub(super) const fn skin_count(&self) -> u32 {
        self.skin_count
    }

    pub(super) const fn logical_morph_count(&self) -> u32 {
        self.logical_morph_count
    }

    pub(super) const fn morph_definition_count(&self) -> u32 {
        self.morph_definition_count
    }

    pub(super) const fn primitive_count(&self) -> u32 {
        self.primitive_count
    }

    pub(super) const fn morph_target_count(&self) -> u32 {
        self.morph_target_count
    }

    pub(super) const fn morph_delta_count(&self) -> u32 {
        self.morph_delta_count
    }

    pub(super) const fn mesh_instance_count(&self) -> u32 {
        self.mesh_instance_count
    }

    pub(super) const fn influence_row_offset_count(&self) -> u32 {
        self.influence_row_offset_count
    }

    pub(super) const fn influence_count(&self) -> u32 {
        self.influence_count
    }

    pub(super) fn validation_work_units(&self) -> Result<u64, StorePlanError> {
        let stored_graph = checked_validation_work_sum([
            optional_record_validation_work(self.skins)?,
            optional_record_validation_work(self.logical_morphs)?,
            optional_record_validation_work(self.morph_definitions)?,
            optional_record_validation_work(self.primitives)?,
            optional_record_validation_work(self.morph_targets)?,
            optional_record_validation_work(self.morph_deltas)?,
            optional_record_validation_work(self.mesh_instances)?,
            optional_sequence_validation_work(self.joint_nodes)?,
            optional_sequence_validation_work(self.inverse_bind_matrices)?,
            optional_sequence_validation_work(self.skin_correction_matrices)?,
            optional_utf8_validation_work(self.skin_names)?,
            optional_sequence_validation_work(self.influence_row_offsets)?,
            optional_sequence_validation_work(self.influence_joints)?,
            optional_sequence_validation_work(self.influence_weights)?,
            optional_sequence_validation_work(self.influence_source_orders)?,
            optional_sequence_validation_work(self.morph_defaults)?,
            optional_utf8_validation_work(self.morph_names)?,
            optional_sequence_validation_work(self.morph_values)?,
            optional_sequence_validation_work(self.morph_sparse_indices)?,
            optional_sequence_validation_work(self.instance_morph_weights)?,
            record_validation_work(self.directory)?,
        ])?;
        let morph_definition_membership = u64::from(self.morph_definition_count);
        let morph_delta_graph_navigation = u64::from(self.primitive_count)
            .checked_add(u64::from(self.morph_target_count))
            .ok_or(StorePlanError::SizeOverflow)?;
        let morph_target_graph_navigation = u64::from(self.primitive_count);
        let influence_source_order_uniqueness = u64::from(self.influence_count);
        let cross_domain_palette_graph = checked_validation_work_sum([
            u64::from(self.logical_morph_count),
            u64::from(self.primitive_count),
            u64::from(self.influence_count),
            u64::from(self.mesh_instance_count),
        ])?;
        let graph_algorithms = checked_validation_work_sum([
            morph_definition_membership,
            morph_delta_graph_navigation,
            morph_target_graph_navigation,
            influence_source_order_uniqueness,
            cross_domain_palette_graph,
        ])?;
        stored_graph
            .checked_add(graph_algorithms)
            .ok_or(StorePlanError::SizeOverflow)
    }

    fn matches_batch<R>(&self, batch: &ValidatedDeformationBatch<'_, R>) -> bool {
        self.skin_count == batch.skin_count()
            && self.logical_morph_count == batch.logical_morph_count()
            && self.morph_definition_count == batch.morph_definition_count()
            && self.primitive_count == batch.primitive_count()
            && self.morph_target_count == batch.morph_target_count()
            && self.morph_delta_count == batch.morph_delta_count()
            && self.mesh_instance_count == batch.mesh_instance_count()
            && self.joint_node_count == batch.joint_node_count()
            && self.inverse_bind_scalar_count == batch.inverse_bind_scalar_count()
            && self.skin_correction_scalar_count == batch.skin_correction_scalar_count()
            && self.skin_name_byte_count == batch.skin_name_byte_count()
            && self.influence_row_offset_count == batch.influence_row_offset_count()
            && self.influence_count == batch.influence_count()
            && self.morph_default_weight_count == batch.morph_default_weight_count()
            && self.morph_name_byte_count == batch.morph_name_byte_count()
            && self.morph_value_count == batch.morph_value_count()
            && self.morph_sparse_index_count == batch.morph_sparse_index_count()
            && self.instance_morph_weight_count == batch.instance_morph_weight_count()
            && self.payload_block_count == batch.payload_block_count()
    }
}

#[derive(Clone, Copy)]
pub(super) struct DeformationDependencyTableRefs {
    node_count: u32,
    logical_mesh_count: u32,
    primitive_count: u32,
    mesh_instance_count: u32,
}
impl DeformationDependencyTableRefs {
    pub(super) fn new(
        nodes: &NodeStorage,
        geometry: &GeometryStorage,
    ) -> Result<Self, StoreRuntimeError> {
        Ok(Self {
            node_count: u32::try_from(nodes.nodes.len())
                .map_err(|_| StoreRuntimeError::InvalidSourceSlot)?,
            logical_mesh_count: u32::try_from(geometry.logical_meshes.len())
                .map_err(|_| StoreRuntimeError::InvalidSourceSlot)?,
            primitive_count: u32::try_from(geometry.primitives.len())
                .map_err(|_| StoreRuntimeError::InvalidSourceSlot)?,
            mesh_instance_count: u32::try_from(geometry.mesh_instances.len())
                .map_err(|_| StoreRuntimeError::InvalidSourceSlot)?,
        })
    }
    const fn matches_plans(self, plans: &DeformationMaterializationPlans) -> bool {
        self.node_count != 0
            && self.logical_mesh_count == plans.logical_morph_count
            && self.primitive_count == plans.primitive_count
            && self.mesh_instance_count == plans.mesh_instance_count
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct RetainedDeformationRefs {
    pub(in crate::canonical_scene::store) directory: CanonicalBlockRef<DeformationDirectorySchema>,
    pub(in crate::canonical_scene::store) tables: DeformationTableRefs,
    pub(in crate::canonical_scene::store) payloads: DeformationPayloadRefs,
}

impl RetainedDeformationRefs {
    pub(super) const fn directory(self) -> CanonicalBlockRef<DeformationDirectorySchema> {
        self.directory
    }

    pub(super) const fn mesh_instances(
        self,
    ) -> Option<CanonicalBlockRef<DeformationMeshInstanceTableSchema>> {
        self.tables.mesh_instances
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct DeformationTableRefs {
    pub(in crate::canonical_scene::store) skins:
        Option<CanonicalBlockRef<DeformationSkinTableSchema>>,
    pub(in crate::canonical_scene::store) logical_morphs:
        Option<CanonicalBlockRef<DeformationLogicalMorphTableSchema>>,
    pub(in crate::canonical_scene::store) morph_definitions:
        Option<CanonicalBlockRef<DeformationMorphDefinitionTableSchema>>,
    pub(in crate::canonical_scene::store) primitives:
        Option<CanonicalBlockRef<DeformationPrimitiveTableSchema>>,
    pub(in crate::canonical_scene::store) morph_targets:
        Option<CanonicalBlockRef<DeformationMorphTargetTableSchema>>,
    pub(in crate::canonical_scene::store) morph_deltas:
        Option<CanonicalBlockRef<DeformationMorphDeltaTableSchema>>,
    pub(in crate::canonical_scene::store) mesh_instances:
        Option<CanonicalBlockRef<DeformationMeshInstanceTableSchema>>,
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct DeformationPayloadRefs {
    pub(in crate::canonical_scene::store) joint_nodes:
        Option<CanonicalBlockRef<DeformationSkinJointNodeSchema>>,
    pub(in crate::canonical_scene::store) inverse_bind_matrices:
        Option<CanonicalBlockRef<DeformationInverseBindMatrixSchema>>,
    pub(in crate::canonical_scene::store) skin_correction_matrices:
        Option<CanonicalBlockRef<DeformationSkinCorrectionMatrixSchema>>,
    pub(in crate::canonical_scene::store) skin_names:
        Option<CanonicalBlockRef<DeformationSkinNameSchema>>,
    pub(in crate::canonical_scene::store) influence_row_offsets:
        Option<CanonicalBlockRef<DeformationInfluenceRowOffsetSchema>>,
    pub(in crate::canonical_scene::store) influence_joints:
        Option<CanonicalBlockRef<DeformationInfluenceJointSchema>>,
    pub(in crate::canonical_scene::store) influence_weights:
        Option<CanonicalBlockRef<DeformationInfluenceWeightSchema>>,
    pub(in crate::canonical_scene::store) influence_source_orders:
        Option<CanonicalBlockRef<DeformationInfluenceSourceOrderSchema>>,
    pub(in crate::canonical_scene::store) morph_defaults:
        Option<CanonicalBlockRef<DeformationMorphDefaultWeightSchema>>,
    pub(in crate::canonical_scene::store) morph_names:
        Option<CanonicalBlockRef<DeformationMorphNameSchema>>,
    pub(in crate::canonical_scene::store) morph_values:
        Option<CanonicalBlockRef<DeformationMorphValueSchema>>,
    pub(in crate::canonical_scene::store) morph_sparse_indices:
        Option<CanonicalBlockRef<DeformationMorphSparseIndexSchema>>,
    pub(in crate::canonical_scene::store) instance_morph_weights:
        Option<CanonicalBlockRef<DeformationInstanceMorphWeightSchema>>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn retain_deformation<'lane, O, PP, U, L, R, NR>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    nodes: &ValidatedNodeBatch<'_, NR>,
    dependencies: DeformationDependencyTableRefs,
    failure: StoreBuildError,
) -> Result<RetainedDeformationRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if !plans.matches_batch(batch) || !dependencies.matches_plans(plans) {
        return Err(StoreRuntimeError::Build(failure));
    }

    let payloads = retain_deformation_payloads(lane, plans, batch, nodes, failure)?;
    let skins = match plans.skins {
        Some(plan) => Some(retain_skin_table(
            lane,
            plan,
            plans,
            batch,
            dependencies.node_count,
            nodes,
            payloads,
            failure,
        )?),
        None => None,
    };
    let morph_definitions = match plans.morph_definitions {
        Some(plan) => Some(retain_morph_definition_table(
            lane, plan, plans, batch, payloads, failure,
        )?),
        None => None,
    };
    let morph_deltas = match plans.morph_deltas {
        Some(plan) => Some(retain_morph_delta_table(
            lane, plan, plans, batch, payloads, failure,
        )?),
        None => None,
    };
    let morph_targets = match plans.morph_targets {
        Some(plan) => Some(retain_morph_target_table(
            lane,
            plan,
            plans,
            batch,
            morph_deltas,
            failure,
        )?),
        None => None,
    };
    let primitives = match plans.primitives {
        Some(plan) => Some(retain_primitive_table(
            lane,
            plan,
            plans,
            batch,
            morph_targets,
            payloads,
            failure,
        )?),
        None => None,
    };
    let logical_morphs = match plans.logical_morphs {
        Some(plan) => Some(retain_logical_morph_table(
            lane,
            plan,
            plans,
            batch,
            morph_definitions,
            payloads,
            failure,
        )?),
        None => None,
    };
    let mesh_instances = match plans.mesh_instances {
        Some(plan) => Some(retain_mesh_instance_table(
            lane, plan, plans, batch, skins, payloads, failure,
        )?),
        None => None,
    };
    let tables = DeformationTableRefs {
        skins,
        logical_morphs,
        morph_definitions,
        primitives,
        morph_targets,
        morph_deltas,
        mesh_instances,
    };
    let directory = retain_deformation_directory(lane, plans, tables, payloads, failure)?;
    Ok(RetainedDeformationRefs {
        directory,
        tables,
        payloads,
    })
}

#[allow(clippy::too_many_arguments)]
fn retain_deformation_payloads<'lane, O, PP, U, L, R, NR>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    nodes: &ValidatedNodeBatch<'_, NR>,
    failure: StoreBuildError,
) -> Result<DeformationPayloadRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let joint_nodes = match plans.joint_nodes {
        Some(plan) => Some(retain_joint_nodes(
            lane, plan, plans, batch, nodes, failure,
        )?),
        None => None,
    };
    let inverse_bind_matrices = match plans.inverse_bind_matrices {
        Some(plan) => Some(retain_inverse_bind_matrices(
            lane, plan, plans, batch, failure,
        )?),
        None => None,
    };
    let skin_correction_matrices = match plans.skin_correction_matrices {
        Some(plan) => Some(retain_skin_correction_matrices(
            lane, plan, plans, batch, failure,
        )?),
        None => None,
    };
    let skin_names = match plans.skin_names {
        Some(plan) => Some(retain_skin_names(lane, plan, plans, batch, failure)?),
        None => None,
    };
    let influence_row_offsets = match plans.influence_row_offsets {
        Some(plan) => Some(retain_influence_row_offsets(
            lane, plan, plans, batch, failure,
        )?),
        None => None,
    };
    let influence_joints = match plans.influence_joints {
        Some(plan) => Some(retain_influence_joints(lane, plan, plans, batch, failure)?),
        None => None,
    };
    let influence_weights = match plans.influence_weights {
        Some(plan) => Some(retain_influence_weights(lane, plan, plans, batch, failure)?),
        None => None,
    };
    let influence_source_orders = match plans.influence_source_orders {
        Some(plan) => Some(retain_influence_source_orders(
            lane, plan, plans, batch, failure,
        )?),
        None => None,
    };
    let morph_defaults = match plans.morph_defaults {
        Some(plan) => Some(retain_morph_defaults(lane, plan, plans, batch, failure)?),
        None => None,
    };
    let morph_names = match plans.morph_names {
        Some(plan) => Some(retain_morph_names(lane, plan, plans, batch, failure)?),
        None => None,
    };
    let morph_values = match plans.morph_values {
        Some(plan) => Some(retain_morph_values(lane, plan, plans, batch, failure)?),
        None => None,
    };
    let morph_sparse_indices = match plans.morph_sparse_indices {
        Some(plan) => Some(retain_morph_sparse_indices(
            lane, plan, plans, batch, failure,
        )?),
        None => None,
    };
    let instance_morph_weights = match plans.instance_morph_weights {
        Some(plan) => Some(retain_instance_morph_weights(
            lane, plan, plans, batch, failure,
        )?),
        None => None,
    };
    Ok(DeformationPayloadRefs {
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
    })
}

#[allow(clippy::too_many_arguments)]
fn retain_joint_nodes<'lane, O, PP, U, L, R, NR>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: SequencePlan<u32>,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    nodes: &ValidatedNodeBatch<'_, NR>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationSkinJointNodeSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, DeformationSkinJointNodeSchema, |control| {
        let skin_count = usize::try_from(plans.skin_count).map_err(|_| failure)?;
        let mapping = nodes.canonical_index_by_local();
        let mut cursor = 0_u32;
        for skin_index in 0..skin_count {
            let skin = batch.skin(skin_index).ok_or(failure)?;
            let authored = skin.authored();
            let count = u32::try_from(authored.joints().len()).map_err(|_| failure)?;
            if skin.joint_offset() != cursor || skin.joint_count() != count {
                return Err(failure.into());
            }
            for joint in authored.joints() {
                let local = usize::try_from(joint.index()).map_err(|_| failure)?;
                let canonical = *mapping.get(local).ok_or(failure)?;
                control.push(canonical)?;
            }
            cursor = cursor.checked_add(count).ok_or(failure)?;
        }
        if cursor != plans.joint_node_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_inverse_bind_matrices<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: SequencePlan<f32>,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationInverseBindMatrixSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, DeformationInverseBindMatrixSchema, |control| {
        let skin_count = usize::try_from(plans.skin_count).map_err(|_| failure)?;
        let mut cursor = 0_u32;
        for skin_index in 0..skin_count {
            let skin = batch.skin(skin_index).ok_or(failure)?;
            if skin.inverse_bind_scalar_offset() != cursor {
                return Err(failure.into());
            }
            for matrix in skin.authored().inverse_bind_matrices() {
                for column in matrix.cols {
                    for scalar in column {
                        control.push(scalar)?;
                    }
                }
            }
            let count = skin
                .joint_count()
                .checked_mul(MATRIX_SCALAR_COUNT)
                .ok_or(failure)?;
            cursor = cursor.checked_add(count).ok_or(failure)?;
        }
        if cursor != plans.inverse_bind_scalar_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_skin_correction_matrices<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: SequencePlan<f32>,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationSkinCorrectionMatrixSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, DeformationSkinCorrectionMatrixSchema, |control| {
        let skin_count = usize::try_from(plans.skin_count).map_err(|_| failure)?;
        let mut cursor = 0_u32;
        for skin_index in 0..skin_count {
            let skin = batch.skin(skin_index).ok_or(failure)?;
            match skin.authored().matrix_spaces().correction() {
                None => {
                    if skin.correction_matrix_scalar_offset() != 0
                        || skin.correction_matrix_scalar_count() != 0
                    {
                        return Err(failure.into());
                    }
                }
                Some(correction) => {
                    if skin.correction_matrix_scalar_offset() != cursor
                        || skin.correction_matrix_scalar_count() != MATRIX_SCALAR_COUNT
                    {
                        return Err(failure.into());
                    }
                    for column in correction.matrix().cols {
                        for scalar in column {
                            control.push(scalar)?;
                        }
                    }
                    cursor = cursor.checked_add(MATRIX_SCALAR_COUNT).ok_or(failure)?;
                }
            }
        }
        if cursor != plans.skin_correction_scalar_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_skin_names<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: Utf8Plan,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationSkinNameSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_utf8(plan, DeformationSkinNameSchema, |control| {
        let skin_count = usize::try_from(plans.skin_count).map_err(|_| failure)?;
        let mut cursor = 0_u32;
        for skin_index in 0..skin_count {
            let skin = batch.skin(skin_index).ok_or(failure)?;
            match skin.authored().name() {
                Presence::Absent => {
                    if skin.name_byte_offset() != 0 || skin.name_byte_count() != 0 {
                        return Err(failure.into());
                    }
                }
                Presence::Present(name) => {
                    let value = name;
                    let count = u32::try_from(value.len()).map_err(|_| failure)?;
                    if skin.name_byte_offset() != cursor || skin.name_byte_count() != count {
                        return Err(failure.into());
                    }
                    control.write_str(value)?;
                    cursor = cursor.checked_add(count).ok_or(failure)?;
                }
            }
        }
        if cursor != plans.skin_name_byte_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_influence_row_offsets<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: SequencePlan<u32>,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationInfluenceRowOffsetSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, DeformationInfluenceRowOffsetSchema, |control| {
        let primitive_count = usize::try_from(plans.primitive_count).map_err(|_| failure)?;
        let mut cursor = 0_u32;
        for primitive_index in 0..primitive_count {
            let primitive = batch.primitive(primitive_index).ok_or(failure)?;
            match primitive.authored().influences() {
                Presence::Absent => {
                    if primitive.influence_row_offset_offset() != 0
                        || primitive.influence_row_offset_count() != 0
                    {
                        return Err(failure.into());
                    }
                }
                Presence::Present(rows) => {
                    let count = u32::try_from(rows.vertex_offsets().len()).map_err(|_| failure)?;
                    if primitive.influence_row_offset_offset() != cursor
                        || primitive.influence_row_offset_count() != count
                    {
                        return Err(failure.into());
                    }
                    for offset in rows.vertex_offsets().iter().copied() {
                        control.push(offset)?;
                    }
                    cursor = cursor.checked_add(count).ok_or(failure)?;
                }
            }
        }
        if cursor != plans.influence_row_offset_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_influence_joints<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: SequencePlan<u32>,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationInfluenceJointSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, DeformationInfluenceJointSchema, |control| {
        let primitive_count = usize::try_from(plans.primitive_count).map_err(|_| failure)?;
        let mut cursor = 0_u32;
        for primitive_index in 0..primitive_count {
            let primitive = batch.primitive(primitive_index).ok_or(failure)?;
            match primitive.authored().influences() {
                Presence::Absent => {
                    if primitive.influence_offset() != 0 || primitive.influence_count() != 0 {
                        return Err(failure.into());
                    }
                }
                Presence::Present(rows) => {
                    let count = u32::try_from(rows.influences().len()).map_err(|_| failure)?;
                    if primitive.influence_offset() != cursor
                        || primitive.influence_count() != count
                    {
                        return Err(failure.into());
                    }
                    for influence in rows.influences().iter().copied() {
                        control.push(influence.joint())?;
                    }
                    cursor = cursor.checked_add(count).ok_or(failure)?;
                }
            }
        }
        if cursor != plans.influence_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_influence_weights<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: SequencePlan<f32>,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationInfluenceWeightSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, DeformationInfluenceWeightSchema, |control| {
        let primitive_count = usize::try_from(plans.primitive_count).map_err(|_| failure)?;
        let mut cursor = 0_u32;
        for primitive_index in 0..primitive_count {
            let primitive = batch.primitive(primitive_index).ok_or(failure)?;
            match primitive.authored().influences() {
                Presence::Absent => {
                    if primitive.influence_offset() != 0 || primitive.influence_count() != 0 {
                        return Err(failure.into());
                    }
                }
                Presence::Present(rows) => {
                    let count = u32::try_from(rows.influences().len()).map_err(|_| failure)?;
                    if primitive.influence_offset() != cursor
                        || primitive.influence_count() != count
                    {
                        return Err(failure.into());
                    }
                    for influence in rows.influences().iter().copied() {
                        control.push(influence.weight())?;
                    }
                    cursor = cursor.checked_add(count).ok_or(failure)?;
                }
            }
        }
        if cursor != plans.influence_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_influence_source_orders<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: SequencePlan<u32>,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationInfluenceSourceOrderSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, DeformationInfluenceSourceOrderSchema, |control| {
        let primitive_count = usize::try_from(plans.primitive_count).map_err(|_| failure)?;
        let mut cursor = 0_u32;
        for primitive_index in 0..primitive_count {
            let primitive = batch.primitive(primitive_index).ok_or(failure)?;
            match primitive.authored().influences() {
                Presence::Absent => {
                    if primitive.influence_offset() != 0 || primitive.influence_count() != 0 {
                        return Err(failure.into());
                    }
                }
                Presence::Present(rows) => {
                    let count = u32::try_from(rows.influences().len()).map_err(|_| failure)?;
                    if primitive.influence_offset() != cursor
                        || primitive.influence_count() != count
                    {
                        return Err(failure.into());
                    }
                    for influence in rows.influences().iter().copied() {
                        control.push(influence.source_order())?;
                    }
                    cursor = cursor.checked_add(count).ok_or(failure)?;
                }
            }
        }
        if cursor != plans.influence_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_morph_defaults<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: SequencePlan<f32>,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationMorphDefaultWeightSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, DeformationMorphDefaultWeightSchema, |control| {
        let logical_count = usize::try_from(plans.logical_morph_count).map_err(|_| failure)?;
        let mut cursor = 0_u32;
        for logical_index in 0..logical_count {
            let logical = batch.logical_morph(logical_index).ok_or(failure)?;
            let defaults = logical.authored().default_weights();
            let count = u32::try_from(defaults.len()).map_err(|_| failure)?;
            if logical.default_weight_offset() != cursor || logical.target_count() != count {
                return Err(failure.into());
            }
            for value in defaults.iter().copied() {
                control.push(value)?;
            }
            cursor = cursor.checked_add(count).ok_or(failure)?;
        }
        if cursor != plans.morph_default_weight_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_morph_names<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: Utf8Plan,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationMorphNameSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_utf8(plan, DeformationMorphNameSchema, |control| {
        let definition_count =
            usize::try_from(plans.morph_definition_count).map_err(|_| failure)?;
        let mut cursor = 0_u32;
        for definition_index in 0..definition_count {
            let definition = batch.morph_definition(definition_index).ok_or(failure)?;
            match definition.authored().name() {
                Presence::Absent => {
                    if definition.name_byte_offset() != 0 || definition.name_byte_count() != 0 {
                        return Err(failure.into());
                    }
                }
                Presence::Present(name) => {
                    let value = name;
                    let count = u32::try_from(value.len()).map_err(|_| failure)?;
                    if definition.name_byte_offset() != cursor
                        || definition.name_byte_count() != count
                    {
                        return Err(failure.into());
                    }
                    control.write_str(value)?;
                    cursor = cursor.checked_add(count).ok_or(failure)?;
                }
            }
        }
        if cursor != plans.morph_name_byte_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_morph_values<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: SequencePlan<f32>,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationMorphValueSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, DeformationMorphValueSchema, |control| {
        let primitive_count = usize::try_from(plans.primitive_count).map_err(|_| failure)?;
        let mut target_cursor = 0_u32;
        let mut delta_cursor = 0_u32;
        let mut value_cursor = 0_u32;
        for primitive_index in 0..primitive_count {
            let primitive = batch.primitive(primitive_index).ok_or(failure)?;
            for target_index in 0..primitive.morph_target_count() {
                let target = primitive.morph_target(target_index).ok_or(failure)?;
                if target.primitive_index()
                    != u32::try_from(primitive_index).map_err(|_| failure)?
                    || target.target_ordinal()
                        != u32::try_from(target_index).map_err(|_| failure)?
                {
                    return Err(failure.into());
                }
                for delta_index in 0..target.delta_count() {
                    let delta = target.delta(delta_index).ok_or(failure)?;
                    let values = match delta.authored_payload() {
                        MorphDeltaPayloadAuthoring::Dense { values }
                        | MorphDeltaPayloadAuthoring::Sparse { values, .. } => values,
                    };
                    let count = u32::try_from(values.len()).map_err(|_| failure)?;
                    if delta.value_offset() != value_cursor || delta.value_count() != count {
                        return Err(failure.into());
                    }
                    for value in values.iter().copied() {
                        control.push(value)?;
                    }
                    value_cursor = value_cursor.checked_add(count).ok_or(failure)?;
                    delta_cursor = delta_cursor.checked_add(1).ok_or(failure)?;
                }
                target_cursor = target_cursor.checked_add(1).ok_or(failure)?;
            }
        }
        if target_cursor != plans.morph_target_count
            || delta_cursor != plans.morph_delta_count
            || value_cursor != plans.morph_value_count
        {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_morph_sparse_indices<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: SequencePlan<u32>,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationMorphSparseIndexSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, DeformationMorphSparseIndexSchema, |control| {
        let primitive_count = usize::try_from(plans.primitive_count).map_err(|_| failure)?;
        let mut target_cursor = 0_u32;
        let mut delta_cursor = 0_u32;
        let mut index_cursor = 0_u32;
        for primitive_index in 0..primitive_count {
            let primitive = batch.primitive(primitive_index).ok_or(failure)?;
            for target_index in 0..primitive.morph_target_count() {
                let target = primitive.morph_target(target_index).ok_or(failure)?;
                for delta_index in 0..target.delta_count() {
                    let delta = target.delta(delta_index).ok_or(failure)?;
                    let indices = match delta.authored_payload() {
                        MorphDeltaPayloadAuthoring::Dense { .. } => &[][..],
                        MorphDeltaPayloadAuthoring::Sparse { indices, .. } => indices,
                    };
                    let count = u32::try_from(indices.len()).map_err(|_| failure)?;
                    if delta.sparse_index_offset() != index_cursor
                        || delta.sparse_index_count() != count
                    {
                        return Err(failure.into());
                    }
                    for index in indices.iter().copied() {
                        control.push(index)?;
                    }
                    index_cursor = index_cursor.checked_add(count).ok_or(failure)?;
                    delta_cursor = delta_cursor.checked_add(1).ok_or(failure)?;
                }
                target_cursor = target_cursor.checked_add(1).ok_or(failure)?;
            }
        }
        if target_cursor != plans.morph_target_count
            || delta_cursor != plans.morph_delta_count
            || index_cursor != plans.morph_sparse_index_count
        {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_instance_morph_weights<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: SequencePlan<f32>,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationInstanceMorphWeightSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, DeformationInstanceMorphWeightSchema, |control| {
        let instance_count = usize::try_from(plans.mesh_instance_count).map_err(|_| failure)?;
        let mut cursor = 0_u32;
        for instance_index in 0..instance_count {
            let instance = batch.mesh_instance(instance_index).ok_or(failure)?;
            match instance.authored().morph_weights() {
                Presence::Absent => {
                    if instance.morph_weight_offset() != 0 || instance.morph_weight_count() != 0 {
                        return Err(failure.into());
                    }
                }
                Presence::Present(weights) => {
                    let count = u32::try_from(weights.len()).map_err(|_| failure)?;
                    if instance.morph_weight_offset() != cursor
                        || instance.morph_weight_count() != count
                    {
                        return Err(failure.into());
                    }
                    for weight in weights.iter().copied() {
                        control.push(weight)?;
                    }
                    cursor = cursor.checked_add(count).ok_or(failure)?;
                }
            }
        }
        if cursor != plans.instance_morph_weight_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_skin_table<'lane, O, PP, U, L, R, NR>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    node_count: u32,
    nodes: &ValidatedNodeBatch<'_, NR>,
    payloads: DeformationPayloadRefs,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationSkinTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, DeformationSkinTableSchema, |control| {
        let skin_count = usize::try_from(plans.skin_count).map_err(|_| failure)?;
        let mapping = nodes.canonical_index_by_local();
        let mut joint_cursor = 0_u32;
        let mut inverse_cursor = 0_u32;
        let mut correction_cursor = 0_u32;
        let mut name_cursor = 0_u32;
        for skin_index in 0..skin_count {
            let skin = batch.skin(skin_index).ok_or(failure)?;
            let authored = skin.authored();
            let skin_id = u32::try_from(skin_index).map_err(|_| failure)?;
            let joint_count = u32::try_from(authored.joints().len()).map_err(|_| failure)?;
            let inverse_count =
                u32::try_from(authored.inverse_bind_matrices().len()).map_err(|_| failure)?;
            let inverse_scalar_count = inverse_count
                .checked_mul(MATRIX_SCALAR_COUNT)
                .ok_or(failure)?;
            if joint_count != inverse_count
                || skin.joint_offset() != joint_cursor
                || skin.joint_count() != joint_count
                || skin.inverse_bind_scalar_offset() != inverse_cursor
            {
                return Err(failure.into());
            }

            let (root_presence, root_index) = match authored.skeleton_root() {
                Presence::Absent => (PRESENCE_ABSENT, None),
                Presence::Present(root) => {
                    let local = usize::try_from(root.index()).map_err(|_| failure)?;
                    (PRESENCE_PRESENT, Some(*mapping.get(local).ok_or(failure)?))
                }
            };
            let spaces = authored.matrix_spaces();
            let encoded_spaces = encode_deformation_skin_matrix_spaces(spaces);
            let correction = spaces.correction();
            match correction {
                None => {
                    if skin.correction_matrix_scalar_offset() != 0
                        || skin.correction_matrix_scalar_count() != 0
                    {
                        return Err(failure.into());
                    }
                }
                Some(_) => {
                    if skin.correction_matrix_scalar_offset() != correction_cursor
                        || skin.correction_matrix_scalar_count() != MATRIX_SCALAR_COUNT
                    {
                        return Err(failure.into());
                    }
                }
            }
            let (name_presence, name_offset, name_count) = match authored.name() {
                Presence::Absent => {
                    if skin.name_byte_offset() != 0 || skin.name_byte_count() != 0 {
                        return Err(failure.into());
                    }
                    (PRESENCE_ABSENT, 0, 0)
                }
                Presence::Present(name) => {
                    let count = u32::try_from(name.len()).map_err(|_| failure)?;
                    if skin.name_byte_offset() != name_cursor || skin.name_byte_count() != count {
                        return Err(failure.into());
                    }
                    (PRESENCE_PRESENT, name_cursor, count)
                }
            };

            control.write_row(|row| {
                row.write_scalar(skin_id)?;
                row.write_scalar(joint_cursor)?;
                row.write_scalar(joint_count)?;
                write_optional_sequence_reference::<u32, _>(
                    row,
                    joint_count != 0,
                    payloads.joint_nodes,
                    failure,
                )?;
                row.write_scalar(inverse_cursor)?;
                row.write_scalar(inverse_count)?;
                write_optional_sequence_reference::<f32, _>(
                    row,
                    inverse_count != 0,
                    payloads.inverse_bind_matrices,
                    failure,
                )?;
                row.write_scalar(root_presence)?;
                match root_index {
                    Some(root) => row.write_scalar(root)?,
                    None => row.write_scalar(0_u32)?,
                }
                row.write_scalar(node_count)?;
                for encoded in encoded_spaces {
                    row.write_scalar(encoded)?;
                }
                match correction {
                    None => {
                        row.write_scalar(PRESENCE_ABSENT)?;
                        row.write_scalar(0_u8)?;
                        row.write_scalar(0_u8)?;
                        row.write_scalar(0_u32)?;
                        row.write_scalar(0_u32)?;
                        row.write_scalar(0_u8)?;
                    }
                    Some(correction) => {
                        row.write_scalar(PRESENCE_PRESENT)?;
                        row.write_scalar(encode_deformation_skin_space(correction.from()))?;
                        row.write_scalar(encode_deformation_skin_space(correction.to()))?;
                        row.write_scalar(correction_cursor)?;
                        row.write_scalar(MATRIX_SCALAR_COUNT)?;
                        row.write_sequence_ref(
                            payloads
                                .skin_correction_matrices
                                .ok_or(failure)?
                                .into_store(),
                        )?;
                    }
                }
                row.write_scalar(name_presence)?;
                row.write_scalar(name_offset)?;
                row.write_scalar(name_count)?;
                write_optional_utf8_reference(row, name_count != 0, payloads.skin_names, failure)
            })?;

            joint_cursor = joint_cursor.checked_add(joint_count).ok_or(failure)?;
            inverse_cursor = inverse_cursor
                .checked_add(inverse_scalar_count)
                .ok_or(failure)?;
            if correction.is_some() {
                correction_cursor = correction_cursor
                    .checked_add(MATRIX_SCALAR_COUNT)
                    .ok_or(failure)?;
            }
            name_cursor = name_cursor.checked_add(name_count).ok_or(failure)?;
        }
        if joint_cursor != plans.joint_node_count
            || inverse_cursor != plans.inverse_bind_scalar_count
            || correction_cursor != plans.skin_correction_scalar_count
            || name_cursor != plans.skin_name_byte_count
        {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_morph_definition_table<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    payloads: DeformationPayloadRefs,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationMorphDefinitionTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, DeformationMorphDefinitionTableSchema, |control| {
        let count = usize::try_from(plans.morph_definition_count).map_err(|_| failure)?;
        let mut name_cursor = 0_u32;
        for definition_index in 0..count {
            let definition = batch.morph_definition(definition_index).ok_or(failure)?;
            let authored = definition.authored();
            let definition_id = u32::try_from(definition_index).map_err(|_| failure)?;
            let (name_presence, name_offset, name_count) = match authored.name() {
                Presence::Absent => {
                    if definition.name_byte_offset() != 0 || definition.name_byte_count() != 0 {
                        return Err(failure.into());
                    }
                    (PRESENCE_ABSENT, 0, 0)
                }
                Presence::Present(name) => {
                    let count = u32::try_from(name.len()).map_err(|_| failure)?;
                    if definition.name_byte_offset() != name_cursor
                        || definition.name_byte_count() != count
                    {
                        return Err(failure.into());
                    }
                    (PRESENCE_PRESENT, name_cursor, count)
                }
            };
            control.write_row(|row| {
                row.write_scalar(definition_id)?;
                row.write_scalar(definition.logical_mesh())?;
                row.write_scalar(definition.target_ordinal())?;
                row.write_scalar(authored.identity().value())?;
                row.write_scalar(name_presence)?;
                row.write_scalar(name_offset)?;
                row.write_scalar(name_count)?;
                write_optional_utf8_reference(row, name_count != 0, payloads.morph_names, failure)
            })?;
            name_cursor = name_cursor.checked_add(name_count).ok_or(failure)?;
        }
        if name_cursor != plans.morph_name_byte_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_morph_delta_table<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    payloads: DeformationPayloadRefs,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationMorphDeltaTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, DeformationMorphDeltaTableSchema, |control| {
        let primitive_count = usize::try_from(plans.primitive_count).map_err(|_| failure)?;
        let mut target_cursor = 0_u32;
        let mut delta_cursor = 0_u32;
        let mut value_cursor = 0_u32;
        let mut sparse_cursor = 0_u32;
        for primitive_index in 0..primitive_count {
            let primitive = batch.primitive(primitive_index).ok_or(failure)?;
            for target_index in 0..primitive.morph_target_count() {
                let target = primitive.morph_target(target_index).ok_or(failure)?;
                if target.primitive_index()
                    != u32::try_from(primitive_index).map_err(|_| failure)?
                    || target.target_ordinal()
                        != u32::try_from(target_index).map_err(|_| failure)?
                {
                    return Err(failure.into());
                }
                for delta_index in 0..target.delta_count() {
                    let delta = target.delta(delta_index).ok_or(failure)?;
                    if delta.semantic() != delta.authored().semantic()
                        || delta.value_offset() != value_cursor
                        || delta.sparse_index_offset() != sparse_cursor
                    {
                        return Err(failure.into());
                    }
                    let sparse = match delta.authored_payload() {
                        MorphDeltaPayloadAuthoring::Dense { values } => {
                            if delta.payload_kind() != DeformationMorphPayloadKind::Dense
                                || u32::try_from(values.len()).map_err(|_| failure)?
                                    != delta.value_count()
                                || delta.sparse_index_count() != 0
                            {
                                return Err(failure.into());
                            }
                            false
                        }
                        MorphDeltaPayloadAuthoring::Sparse {
                            indices, values, ..
                        } => {
                            if delta.payload_kind() != DeformationMorphPayloadKind::Sparse
                                || u32::try_from(values.len()).map_err(|_| failure)?
                                    != delta.value_count()
                                || u32::try_from(indices.len()).map_err(|_| failure)?
                                    != delta.sparse_index_count()
                            {
                                return Err(failure.into());
                            }
                            true
                        }
                    };
                    control.write_row(|row| {
                        row.write_scalar(delta_cursor)?;
                        row.write_scalar(target_cursor)?;
                        row.write_scalar(encode_deformation_morph_semantic(delta.semantic()))?;
                        row.write_scalar(encode_deformation_morph_payload(sparse))?;
                        row.write_scalar(delta.logical_count())?;
                        row.write_scalar(delta.explicit_count())?;
                        row.write_scalar(delta.value_offset())?;
                        row.write_scalar(delta.value_count())?;
                        write_optional_sequence_reference::<f32, _>(
                            row,
                            delta.value_count() != 0,
                            payloads.morph_values,
                            failure,
                        )?;
                        row.write_scalar(delta.sparse_index_offset())?;
                        row.write_scalar(delta.sparse_index_count())?;
                        write_optional_sequence_reference::<u32, _>(
                            row,
                            delta.sparse_index_count() != 0,
                            payloads.morph_sparse_indices,
                            failure,
                        )
                    })?;
                    value_cursor = value_cursor
                        .checked_add(delta.value_count())
                        .ok_or(failure)?;
                    sparse_cursor = sparse_cursor
                        .checked_add(delta.sparse_index_count())
                        .ok_or(failure)?;
                    delta_cursor = delta_cursor.checked_add(1).ok_or(failure)?;
                }
                target_cursor = target_cursor.checked_add(1).ok_or(failure)?;
            }
        }
        if target_cursor != plans.morph_target_count
            || delta_cursor != plans.morph_delta_count
            || value_cursor != plans.morph_value_count
            || sparse_cursor != plans.morph_sparse_index_count
        {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_morph_target_table<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    deltas: Option<CanonicalBlockRef<DeformationMorphDeltaTableSchema>>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationMorphTargetTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, DeformationMorphTargetTableSchema, |control| {
        let primitive_count = usize::try_from(plans.primitive_count).map_err(|_| failure)?;
        let mut target_cursor = 0_u32;
        let mut delta_cursor = 0_u32;
        for primitive_index in 0..primitive_count {
            let primitive = batch.primitive(primitive_index).ok_or(failure)?;
            for target_index in 0..primitive.morph_target_count() {
                let target = primitive.morph_target(target_index).ok_or(failure)?;
                let primitive_id = u32::try_from(primitive_index).map_err(|_| failure)?;
                let target_ordinal = u32::try_from(target_index).map_err(|_| failure)?;
                let delta_count = u32::try_from(target.delta_count()).map_err(|_| failure)?;
                if target.primitive_index() != primitive_id
                    || target.target_ordinal() != target_ordinal
                {
                    return Err(failure.into());
                }
                control.write_row(|row| {
                    row.write_scalar(target_cursor)?;
                    row.write_scalar(primitive_id)?;
                    row.write_scalar(target_ordinal)?;
                    row.write_scalar(delta_cursor)?;
                    row.write_scalar(delta_count)?;
                    write_optional_record_reference(row, delta_count != 0, deltas, failure)
                })?;
                delta_cursor = delta_cursor.checked_add(delta_count).ok_or(failure)?;
                target_cursor = target_cursor.checked_add(1).ok_or(failure)?;
            }
        }
        if target_cursor != plans.morph_target_count || delta_cursor != plans.morph_delta_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_primitive_table<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    morph_targets: Option<CanonicalBlockRef<DeformationMorphTargetTableSchema>>,
    payloads: DeformationPayloadRefs,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationPrimitiveTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, DeformationPrimitiveTableSchema, |control| {
        let primitive_count = usize::try_from(plans.primitive_count).map_err(|_| failure)?;
        let mut row_offset_cursor = 0_u32;
        let mut influence_cursor = 0_u32;
        let mut target_cursor = 0_u32;
        for primitive_index in 0..primitive_count {
            let primitive = batch.primitive(primitive_index).ok_or(failure)?;
            let primitive_id = u32::try_from(primitive_index).map_err(|_| failure)?;
            let (influence_presence, row_offset, row_count, influence_offset, influence_count) =
                match primitive.authored().influences() {
                    Presence::Absent => {
                        if primitive.influence_row_offset_offset() != 0
                            || primitive.influence_row_offset_count() != 0
                            || primitive.influence_offset() != 0
                            || primitive.influence_count() != 0
                        {
                            return Err(failure.into());
                        }
                        (PRESENCE_ABSENT, 0, 0, 0, 0)
                    }
                    Presence::Present(rows) => {
                        let row_count =
                            u32::try_from(rows.vertex_offsets().len()).map_err(|_| failure)?;
                        let influence_count =
                            u32::try_from(rows.influences().len()).map_err(|_| failure)?;
                        if primitive.influence_row_offset_offset() != row_offset_cursor
                            || primitive.influence_row_offset_count() != row_count
                            || primitive.influence_offset() != influence_cursor
                            || primitive.influence_count() != influence_count
                        {
                            return Err(failure.into());
                        }
                        (
                            PRESENCE_PRESENT,
                            row_offset_cursor,
                            row_count,
                            influence_cursor,
                            influence_count,
                        )
                    }
                };
            let target_count =
                u32::try_from(primitive.morph_target_count()).map_err(|_| failure)?;
            control.write_row(|row| {
                row.write_scalar(primitive_id)?;
                row.write_scalar(influence_presence)?;
                row.write_scalar(row_offset)?;
                row.write_scalar(row_count)?;
                write_optional_sequence_reference::<u32, _>(
                    row,
                    row_count != 0,
                    payloads.influence_row_offsets,
                    failure,
                )?;
                row.write_scalar(influence_offset)?;
                row.write_scalar(influence_count)?;
                write_optional_sequence_reference::<u32, _>(
                    row,
                    influence_count != 0,
                    payloads.influence_joints,
                    failure,
                )?;
                write_optional_sequence_reference::<f32, _>(
                    row,
                    influence_count != 0,
                    payloads.influence_weights,
                    failure,
                )?;
                write_optional_sequence_reference::<u32, _>(
                    row,
                    influence_count != 0,
                    payloads.influence_source_orders,
                    failure,
                )?;
                row.write_scalar(target_cursor)?;
                row.write_scalar(target_count)?;
                write_optional_record_reference(row, target_count != 0, morph_targets, failure)
            })?;
            row_offset_cursor = row_offset_cursor.checked_add(row_count).ok_or(failure)?;
            influence_cursor = influence_cursor
                .checked_add(influence_count)
                .ok_or(failure)?;
            target_cursor = target_cursor.checked_add(target_count).ok_or(failure)?;
        }
        if row_offset_cursor != plans.influence_row_offset_count
            || influence_cursor != plans.influence_count
            || target_cursor != plans.morph_target_count
        {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_logical_morph_table<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    definitions: Option<CanonicalBlockRef<DeformationMorphDefinitionTableSchema>>,
    payloads: DeformationPayloadRefs,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationLogicalMorphTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, DeformationLogicalMorphTableSchema, |control| {
        let logical_count = usize::try_from(plans.logical_morph_count).map_err(|_| failure)?;
        let mut definition_cursor = 0_u32;
        let mut default_cursor = 0_u32;
        for logical_index in 0..logical_count {
            let logical = batch.logical_morph(logical_index).ok_or(failure)?;
            let logical_id = u32::try_from(logical_index).map_err(|_| failure)?;
            let definition_range = logical.definitions();
            let default_count =
                u32::try_from(logical.authored().default_weights().len()).map_err(|_| failure)?;
            if definition_range.offset() != definition_cursor
                || definition_range.count() != logical.target_count()
                || logical.default_weight_offset() != default_cursor
                || default_count != logical.target_count()
            {
                return Err(failure.into());
            }
            control.write_row(|row| {
                row.write_scalar(logical_id)?;
                row.write_scalar(definition_range.offset())?;
                row.write_scalar(definition_range.count())?;
                write_optional_record_reference(
                    row,
                    definition_range.count() != 0,
                    definitions,
                    failure,
                )?;
                row.write_scalar(default_cursor)?;
                row.write_scalar(default_count)?;
                write_optional_sequence_reference::<f32, _>(
                    row,
                    default_count != 0,
                    payloads.morph_defaults,
                    failure,
                )
            })?;
            definition_cursor = definition_cursor
                .checked_add(definition_range.count())
                .ok_or(failure)?;
            default_cursor = default_cursor.checked_add(default_count).ok_or(failure)?;
        }
        if definition_cursor != plans.morph_definition_count
            || default_cursor != plans.morph_default_weight_count
        {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_mesh_instance_table<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    plans: &DeformationMaterializationPlans,
    batch: &ValidatedDeformationBatch<'_, R>,
    skins: Option<CanonicalBlockRef<DeformationSkinTableSchema>>,
    payloads: DeformationPayloadRefs,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationMeshInstanceTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, DeformationMeshInstanceTableSchema, |control| {
        let instance_count = usize::try_from(plans.mesh_instance_count).map_err(|_| failure)?;
        let mut weight_cursor = 0_u32;
        for instance_index in 0..instance_count {
            let instance = batch.mesh_instance(instance_index).ok_or(failure)?;
            let authored = instance.authored();
            let instance_id = u32::try_from(instance_index).map_err(|_| failure)?;
            let (skin_presence, skin_index) = match authored.skin() {
                Presence::Absent => {
                    if instance.skin().is_some() {
                        return Err(failure.into());
                    }
                    (PRESENCE_ABSENT, None)
                }
                Presence::Present(skin) => {
                    if instance.skin() != Some(skin) {
                        return Err(failure.into());
                    }
                    (PRESENCE_PRESENT, Some(skin.index()))
                }
            };
            let (weights_presence, weight_offset, weight_count) = match authored.morph_weights() {
                Presence::Absent => {
                    if instance.morph_weight_offset() != 0 || instance.morph_weight_count() != 0 {
                        return Err(failure.into());
                    }
                    (PRESENCE_ABSENT, 0, 0)
                }
                Presence::Present(weights) => {
                    let count = u32::try_from(weights.len()).map_err(|_| failure)?;
                    if instance.morph_weight_offset() != weight_cursor
                        || instance.morph_weight_count() != count
                    {
                        return Err(failure.into());
                    }
                    (PRESENCE_PRESENT, weight_cursor, count)
                }
            };
            control.write_row(|row| {
                row.write_scalar(instance_id)?;
                row.write_scalar(skin_presence)?;
                match skin_index {
                    Some(index) => row.write_scalar(index)?,
                    None => row.write_scalar(0_u32)?,
                }
                write_optional_record_reference(row, skin_index.is_some(), skins, failure)?;
                row.write_scalar(weights_presence)?;
                row.write_scalar(weight_offset)?;
                row.write_scalar(weight_count)?;
                write_optional_sequence_reference::<f32, _>(
                    row,
                    weight_count != 0,
                    payloads.instance_morph_weights,
                    failure,
                )
            })?;
            weight_cursor = weight_cursor.checked_add(weight_count).ok_or(failure)?;
        }
        if weight_cursor != plans.instance_morph_weight_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_deformation_directory<'lane, O, PP, U, L>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plans: &DeformationMaterializationPlans,
    tables: DeformationTableRefs,
    payloads: DeformationPayloadRefs,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<DeformationDirectorySchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plans.directory, DeformationDirectorySchema, |control| {
        control.write_row(|row| {
            row.write_scalar(DEFORMATION_SCHEMA_REVISION)?;
            write_optional_record_group(row, plans.skin_count, tables.skins, failure)?;
            write_optional_sequence_group::<u32, _>(
                row,
                plans.joint_node_count,
                payloads.joint_nodes,
                failure,
            )?;
            write_optional_sequence_group::<f32, _>(
                row,
                plans.inverse_bind_scalar_count,
                payloads.inverse_bind_matrices,
                failure,
            )?;
            write_optional_record_group(
                row,
                plans.logical_morph_count,
                tables.logical_morphs,
                failure,
            )?;
            write_optional_record_group(row, plans.primitive_count, tables.primitives, failure)?;
            write_optional_sequence_group::<u32, _>(
                row,
                plans.influence_row_offset_count,
                payloads.influence_row_offsets,
                failure,
            )?;
            row.write_scalar(plans.influence_count)?;
            write_optional_sequence_reference::<u32, _>(
                row,
                plans.influence_count != 0,
                payloads.influence_joints,
                failure,
            )?;
            write_optional_sequence_reference::<f32, _>(
                row,
                plans.influence_count != 0,
                payloads.influence_weights,
                failure,
            )?;
            write_optional_sequence_reference::<u32, _>(
                row,
                plans.influence_count != 0,
                payloads.influence_source_orders,
                failure,
            )?;
            write_optional_record_group(
                row,
                plans.morph_target_count,
                tables.morph_targets,
                failure,
            )?;
            write_optional_record_group(
                row,
                plans.morph_delta_count,
                tables.morph_deltas,
                failure,
            )?;
            write_optional_sequence_group::<f32, _>(
                row,
                plans.morph_default_weight_count,
                payloads.morph_defaults,
                failure,
            )?;
            write_optional_sequence_group::<f32, _>(
                row,
                plans.morph_value_count,
                payloads.morph_values,
                failure,
            )?;
            write_optional_sequence_group::<u32, _>(
                row,
                plans.morph_sparse_index_count,
                payloads.morph_sparse_indices,
                failure,
            )?;
            write_optional_record_group(
                row,
                plans.mesh_instance_count,
                tables.mesh_instances,
                failure,
            )?;
            write_optional_sequence_group::<f32, _>(
                row,
                plans.instance_morph_weight_count,
                payloads.instance_morph_weights,
                failure,
            )?;
            write_optional_sequence_group::<f32, _>(
                row,
                plans.skin_correction_scalar_count,
                payloads.skin_correction_matrices,
                failure,
            )?;
            write_optional_record_group(
                row,
                plans.morph_definition_count,
                tables.morph_definitions,
                failure,
            )?;
            write_optional_utf8_group(
                row,
                plans.morph_name_byte_count,
                payloads.morph_names,
                failure,
            )?;
            write_optional_utf8_group(
                row,
                plans.skin_name_byte_count,
                payloads.skin_names,
                failure,
            )
        })?;
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn write_optional_record_reference<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    present: bool,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    if present {
        row.write_record_ref(reference.ok_or(failure)?.into_store())
    } else {
        row.write_scalar(0_u8)
    }
}

fn write_optional_sequence_reference<T, Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    present: bool,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
{
    if present {
        row.write_sequence_ref(reference.ok_or(failure)?.into_store())
    } else {
        row.write_scalar(0_u8)
    }
}

fn write_optional_utf8_reference<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    present: bool,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    if present {
        row.write_utf8_ref(reference.ok_or(failure)?.into_store())
    } else {
        row.write_scalar(0_u8)
    }
}

fn write_optional_record_group<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    row.write_scalar(count)?;
    if reference.is_some() != (count != 0) {
        return Err(failure.into());
    }
    write_optional_record_reference(row, count != 0, reference, failure)
}

fn write_optional_sequence_group<T, Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
{
    row.write_scalar(count)?;
    if reference.is_some() != (count != 0) {
        return Err(failure.into());
    }
    write_optional_sequence_reference::<T, Tag>(row, count != 0, reference, failure)
}

fn write_optional_utf8_group<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    row.write_scalar(count)?;
    if reference.is_some() != (count != 0) {
        return Err(failure.into());
    }
    write_optional_utf8_reference(row, count != 0, reference, failure)
}

fn record_plan_if_nonempty(
    count: u32,
    fields: usize,
) -> Result<Option<RecordPlan>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(RecordPlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
            fields,
        )?))
    }
}

fn sequence_plan_if_nonempty<T: StoredScalar>(
    count: u32,
) -> Result<Option<SequencePlan<T>>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(SequencePlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
        )?))
    }
}

fn utf8_plan_if_nonempty(count: u32) -> Result<Option<Utf8Plan>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(Utf8Plan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
        )?))
    }
}
