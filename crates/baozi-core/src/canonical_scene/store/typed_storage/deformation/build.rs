//! Capacity-qualified construction inside the existing central retained-storage admission.

use baozi_operation::{CapacityPlan, StorePlanError};

use crate::canonical_scene::store::production::input::deformation::{
    InfluenceRowsAuthoring, SkinAuthoring,
};
use crate::canonical_scene::store::production::{
    MorphDeltaPayloadAuthoring, Presence, ValidatedDeformationBatch, ValidatedNodeBatch,
};
use crate::canonical_scene::store::typed_storage::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, append, append_name,
    capacity, finish, push,
};

use super::*;

#[cfg(test)]
mod tests;

const SKINS: usize = 0;
const LOGICAL_MORPHS: usize = 1;
const MORPH_DEFINITIONS: usize = 2;
const PRIMITIVES: usize = 3;
const MORPH_TARGETS: usize = 4;
const MORPH_DELTAS: usize = 5;
const MESH_INSTANCES: usize = 6;
const JOINTS: usize = 7;
const INVERSE_BIND_MATRICES: usize = 8;
const NAMES: usize = 9;
const INFLUENCE_VERTEX_OFFSETS: usize = 10;
const INFLUENCE_JOINTS: usize = 11;
const INFLUENCE_WEIGHTS: usize = 12;
const INFLUENCE_SOURCE_ORDERS: usize = 13;
const MORPH_WEIGHTS: usize = 14;
const MORPH_VALUES: usize = 15;
const MORPH_SPARSE_INDICES: usize = 16;

/// Only the seven row counts and seven independent payload counts are retained. Capacity
/// envelopes and element widths are reconstructed when admission or construction needs them.
/// Materialization performs one action per initialized element, including one action per
/// whole Mat4 and one string action per name byte. Inline correction work belongs to its
/// bounded skin-row action; there is no separate correction allocation or scalar traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct DeformationStoragePlan {
    rows: [u32; 7],
    joint_count: usize,
    name_bytes: usize,
    influence_row_offset_count: usize,
    influence_count: usize,
    morph_weight_count: usize,
    morph_value_count: usize,
    morph_sparse_index_count: usize,
}

impl DeformationStoragePlan {
    pub(in crate::canonical_scene::store) fn new<R>(
        batch: &ValidatedDeformationBatch<'_, R>,
    ) -> Result<Self, StorePlanError> {
        let inverse_bind_scalars = batch.inverse_bind_scalar_count();
        if !inverse_bind_scalars.is_multiple_of(16) {
            return Err(StorePlanError::InvalidBlockTable);
        }
        let names = u64::from(batch.skin_name_byte_count())
            .checked_add(u64::from(batch.morph_name_byte_count()))
            .ok_or(StorePlanError::SizeOverflow)?;
        let morph_weights = u64::from(batch.morph_default_weight_count())
            .checked_add(u64::from(batch.instance_morph_weight_count()))
            .ok_or(StorePlanError::SizeOverflow)?;
        let plan = Self::for_shape(
            [
                u64::from(batch.skin_count()),
                u64::from(batch.logical_morph_count()),
                u64::from(batch.morph_definition_count()),
                u64::from(batch.primitive_count()),
                u64::from(batch.morph_target_count()),
                u64::from(batch.morph_delta_count()),
                u64::from(batch.mesh_instance_count()),
            ],
            [
                u64::from(batch.joint_node_count()),
                u64::from(inverse_bind_scalars / 16),
                names,
                u64::from(batch.influence_row_offset_count()),
                u64::from(batch.influence_count()),
                u64::from(batch.influence_count()),
                u64::from(batch.influence_count()),
                morph_weights,
                u64::from(batch.morph_value_count()),
                u64::from(batch.morph_sparse_index_count()),
            ],
        )?;
        let expected_f32 = [
            u64::from(inverse_bind_scalars),
            u64::from(batch.skin_correction_scalar_count()),
            u64::from(batch.influence_count()),
            morph_weights,
            u64::from(batch.morph_value_count()),
        ]
        .into_iter()
        .try_fold(0_u64, |sum, count| {
            sum.checked_add(count).ok_or(StorePlanError::SizeOverflow)
        })?;
        let expected_u32 = [
            u64::from(batch.joint_node_count()),
            u64::from(batch.influence_row_offset_count()),
            u64::from(batch.influence_count()),
            u64::from(batch.influence_count()),
            u64::from(batch.morph_sparse_index_count()),
        ]
        .into_iter()
        .try_fold(0_u64, |sum, count| {
            sum.checked_add(count).ok_or(StorePlanError::SizeOverflow)
        })?;
        let expected_cells = expected_f32
            .checked_add(expected_u32)
            .and_then(|count| count.checked_add(names))
            .ok_or(StorePlanError::SizeOverflow)?;
        if expected_f32 != batch.f32_scalar_count()
            || expected_u32 != batch.u32_scalar_count()
            || expected_cells != batch.semantic_cell_count()
            || plan.canonical_bytes()? != batch.canonical_bytes()
        {
            return Err(StorePlanError::InvalidBlockTable);
        }
        Ok(plan)
    }

    /// Rows: skins, logical morphs, definitions, primitives, targets, deltas, instances.
    /// Payloads: joint nodes, Mat4 inverse binds, name bytes, influence vertex offsets,
    /// influence joints, weights, source orders, combined morph weights, morph F32 values,
    /// and sparse indices. Counts describe initialized elements, never spare capacity.
    pub(in crate::canonical_scene::store) fn for_shape(
        rows: [u64; 7],
        payloads: [u64; 10],
    ) -> Result<Self, StorePlanError> {
        let mut row_counts = [0; 7];
        for (output, count) in row_counts.iter_mut().zip(rows) {
            *output = u32::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?;
        }
        if payloads[0] != payloads[1]
            || payloads[4] != payloads[5]
            || payloads[4] != payloads[6]
            || !payloads[8].is_multiple_of(3)
        {
            return Err(StorePlanError::InvalidBlockTable);
        }
        let plan = Self {
            rows: row_counts,
            joint_count: usize::try_from(payloads[0]).map_err(|_| StorePlanError::SizeOverflow)?,
            name_bytes: usize::try_from(payloads[2]).map_err(|_| StorePlanError::SizeOverflow)?,
            influence_row_offset_count: usize::try_from(payloads[3])
                .map_err(|_| StorePlanError::SizeOverflow)?,
            influence_count: usize::try_from(payloads[4])
                .map_err(|_| StorePlanError::SizeOverflow)?,
            morph_weight_count: usize::try_from(payloads[7])
                .map_err(|_| StorePlanError::SizeOverflow)?,
            morph_value_count: usize::try_from(payloads[8])
                .map_err(|_| StorePlanError::SizeOverflow)?,
            morph_sparse_index_count: usize::try_from(payloads[9])
                .map_err(|_| StorePlanError::SizeOverflow)?,
        };
        plan.capacities()?;
        Ok(plan)
    }

    pub(in crate::canonical_scene::store) fn for_storage(
        storage: &DeformationStorage,
    ) -> Result<Self, StorePlanError> {
        Self::for_shape(
            lengths([
                storage.skins.len(),
                storage.logical_morphs.len(),
                storage.morph_definitions.len(),
                storage.primitives.len(),
                storage.morph_targets.len(),
                storage.morph_deltas.len(),
                storage.mesh_instances.len(),
            ])?,
            lengths([
                storage.joints.len(),
                storage.inverse_bind_matrices.len(),
                storage.names.len(),
                storage.influence_vertex_offsets.len(),
                storage.influence_joints.len(),
                storage.influence_weights.len(),
                storage.influence_source_orders.len(),
                storage.morph_weights.len(),
                storage.morph_values.len(),
                storage.morph_sparse_indices.len(),
            ])?,
        )
    }

    /// Fixed admission order: seven row tables followed by ten payload buffers.
    /// The checked result keeps reconstruction independent of an infallibility assumption.
    pub(in crate::canonical_scene::store) fn capacities(
        self,
    ) -> Result<[CapacityPlan; 17], StorePlanError> {
        let rows = self.row_counts();
        let payloads = self.payload_counts();
        Ok([
            capacity::<StoredSkin>(rows[0])?,
            capacity::<StoredLogicalMorph>(rows[1])?,
            capacity::<StoredMorphDefinition>(rows[2])?,
            capacity::<StoredPrimitiveDeformation>(rows[3])?,
            capacity::<StoredMorphTarget>(rows[4])?,
            capacity::<StoredMorphDelta>(rows[5])?,
            capacity::<StoredMeshInstanceDeformation>(rows[6])?,
            capacity::<StoredNodeIndex>(payloads[0])?,
            capacity::<Mat4>(payloads[1])?,
            capacity::<u8>(payloads[2])?,
            capacity::<u32>(payloads[3])?,
            capacity::<u32>(payloads[4])?,
            capacity::<f32>(payloads[5])?,
            capacity::<u32>(payloads[6])?,
            capacity::<f32>(payloads[7])?,
            capacity::<f32>(payloads[8])?,
            capacity::<u32>(payloads[9])?,
        ])
    }

    pub(in crate::canonical_scene::store) fn row_counts(self) -> [u64; 7] {
        self.rows.map(u64::from)
    }

    pub(in crate::canonical_scene::store) fn payload_counts(self) -> [u64; 10] {
        [
            self.joint_count as u64,
            self.joint_count as u64,
            self.name_bytes as u64,
            self.influence_row_offset_count as u64,
            self.influence_count as u64,
            self.influence_count as u64,
            self.influence_count as u64,
            self.morph_weight_count as u64,
            self.morph_value_count as u64,
            self.morph_sparse_index_count as u64,
        ]
    }

    pub(in crate::canonical_scene::store) fn name_bytes(self) -> usize {
        self.name_bytes
    }

    /// Exact successful construction work, including string actions. Cancellation or an
    /// invariant failure consumes only the prefix whose actions actually ran.
    pub(in crate::canonical_scene::store) fn work(self) -> Result<u64, StorePlanError> {
        self.row_counts()
            .into_iter()
            .chain(self.payload_counts())
            .try_fold(0_u64, |sum, count| {
                sum.checked_add(count).ok_or(StorePlanError::SizeOverflow)
            })
    }

    pub(in crate::canonical_scene::store) fn canonical_bytes(self) -> Result<u64, StorePlanError> {
        Self::canonical_bytes_for_shape(self.row_counts(), self.payload_counts())
    }

    /// The real typed owner header and used rows/payload elements for this target ABI.
    /// Inline corrections are already part of StoredSkin. Owner infrastructure and unused
    /// allocator capacity belong to retained admission rather than canonical bytes.
    pub(in crate::canonical_scene::store) fn canonical_bytes_for_shape(
        rows: [u64; 7],
        payloads: [u64; 10],
    ) -> Result<u64, StorePlanError> {
        let counts = [
            rows[0],
            rows[1],
            rows[2],
            rows[3],
            rows[4],
            rows[5],
            rows[6],
            payloads[0],
            payloads[1],
            payloads[2],
            payloads[3],
            payloads[4],
            payloads[5],
            payloads[6],
            payloads[7],
            payloads[8],
            payloads[9],
        ];
        let widths = [
            std::mem::size_of::<StoredSkin>(),
            std::mem::size_of::<StoredLogicalMorph>(),
            std::mem::size_of::<StoredMorphDefinition>(),
            std::mem::size_of::<StoredPrimitiveDeformation>(),
            std::mem::size_of::<StoredMorphTarget>(),
            std::mem::size_of::<StoredMorphDelta>(),
            std::mem::size_of::<StoredMeshInstanceDeformation>(),
            std::mem::size_of::<StoredNodeIndex>(),
            std::mem::size_of::<Mat4>(),
            std::mem::size_of::<u8>(),
            std::mem::size_of::<u32>(),
            std::mem::size_of::<u32>(),
            std::mem::size_of::<f32>(),
            std::mem::size_of::<u32>(),
            std::mem::size_of::<f32>(),
            std::mem::size_of::<f32>(),
            std::mem::size_of::<u32>(),
        ];
        let header = u64::try_from(std::mem::size_of::<DeformationStorage>())
            .map_err(|_| StorePlanError::SizeOverflow)?;
        counts
            .into_iter()
            .zip(widths)
            .try_fold(header, |sum, (count, width)| {
                let width = u64::try_from(width).map_err(|_| StorePlanError::SizeOverflow)?;
                count
                    .checked_mul(width)
                    .and_then(|bytes| sum.checked_add(bytes))
                    .ok_or(StorePlanError::SizeOverflow)
            })
    }

    /// The lane must admit all seventeen capacities and the owner before entering build.
    /// Canonical node rows come from the same sealed node batch used to build NodeStorage.
    pub(in crate::canonical_scene::store) fn build<R, NR, C: TypedStorageBuildControl>(
        self,
        batch: &ValidatedDeformationBatch<'_, R>,
        nodes: &ValidatedNodeBatch<'_, NR>,
        mut control: C,
    ) -> BuildResult<DeformationStorage, C::Error> {
        if Self::new(batch).map_err(|_| TypedStorageBuildError::Invariant)? != self {
            return Err(TypedStorageBuildError::Invariant);
        }
        let builder = DeformationStorageBuilder {
            capacities: self
                .capacities()
                .map_err(|_| TypedStorageBuildError::Invariant)?,
        };
        let mut storage = builder.allocate()?;
        for index in 0..batch.skin_count() as usize {
            let skin = batch.skin(index).ok_or(TypedStorageBuildError::Invariant)?;
            builder.append_skin(
                &mut storage,
                skin.authored(),
                nodes.canonical_index_by_local(),
                &mut control,
            )?;
        }
        for index in 0..batch.logical_morph_count() as usize {
            let logical = batch
                .logical_morph(index)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let definitions = logical.definitions();
            let default_weights = append(
                &mut storage.morph_weights,
                builder.capacities[MORPH_WEIGHTS],
                logical.authored().default_weights(),
                &mut control,
            )?;
            push(
                &mut storage.logical_morphs,
                builder.capacities[LOGICAL_MORPHS],
                StoredLogicalMorph {
                    definitions: TableRange::new(
                        definitions.offset() as usize,
                        definitions.count() as usize,
                    ),
                    default_weights,
                },
                &mut control,
            )?;
        }
        for index in 0..batch.morph_definition_count() as usize {
            let definition = batch
                .morph_definition(index)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let name = append_name(
                &mut storage.names,
                builder.capacities[NAMES],
                definition.authored().name(),
                &mut control,
            )?;
            push(
                &mut storage.morph_definitions,
                builder.capacities[MORPH_DEFINITIONS],
                StoredMorphDefinition {
                    logical_mesh: StoredLogicalMeshIndex(definition.logical_mesh()),
                    target_ordinal: definition.target_ordinal(),
                    stable_identity: definition.authored().identity(),
                    name,
                },
                &mut control,
            )?;
        }
        for index in 0..batch.primitive_count() as usize {
            let primitive = batch
                .primitive(index)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let influences = match primitive.authored().influences() {
                Presence::Absent => None,
                Presence::Present(rows) => {
                    Some(builder.append_influences(&mut storage, rows, &mut control)?)
                }
            };
            let target_start = storage.morph_targets.len();
            for ordinal in 0..primitive.morph_target_count() {
                let target = primitive
                    .morph_target(ordinal)
                    .ok_or(TypedStorageBuildError::Invariant)?;
                if target.primitive_index() as usize != index
                    || target.target_ordinal() as usize != ordinal
                {
                    return Err(TypedStorageBuildError::Invariant);
                }
                let target_index = StoredMorphTargetIndex(
                    u32::try_from(storage.morph_targets.len())
                        .map_err(|_| TypedStorageBuildError::Invariant)?,
                );
                let delta_start = storage.morph_deltas.len();
                for delta_index in 0..target.delta_count() {
                    let delta = target
                        .delta(delta_index)
                        .ok_or(TypedStorageBuildError::Invariant)?;
                    let payload = builder.append_morph_payload(
                        &mut storage,
                        delta.authored_payload(),
                        &mut control,
                    )?;
                    push(
                        &mut storage.morph_deltas,
                        builder.capacities[MORPH_DELTAS],
                        StoredMorphDelta {
                            target: target_index,
                            semantic: delta.semantic(),
                            payload,
                        },
                        &mut control,
                    )?;
                }
                push(
                    &mut storage.morph_targets,
                    builder.capacities[MORPH_TARGETS],
                    StoredMorphTarget {
                        primitive: target.primitive_index(),
                        target_ordinal: target.target_ordinal(),
                        deltas: TableRange::new(delta_start, target.delta_count()),
                    },
                    &mut control,
                )?;
            }
            push(
                &mut storage.primitives,
                builder.capacities[PRIMITIVES],
                StoredPrimitiveDeformation {
                    influences,
                    morph_targets: TableRange::new(target_start, primitive.morph_target_count()),
                },
                &mut control,
            )?;
        }
        for index in 0..batch.mesh_instance_count() as usize {
            let instance = batch
                .mesh_instance(index)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let morph_weights = match instance.authored().morph_weights() {
                Presence::Absent => None,
                Presence::Present(weights) => Some(append(
                    &mut storage.morph_weights,
                    builder.capacities[MORPH_WEIGHTS],
                    weights,
                    &mut control,
                )?),
            };
            push(
                &mut storage.mesh_instances,
                builder.capacities[MESH_INSTANCES],
                StoredMeshInstanceDeformation {
                    skin: instance.skin().map(|skin| StoredSkinIndex(skin.index())),
                    morph_weights,
                },
                &mut control,
            )?;
        }
        builder.finish(&storage)?;
        Ok(storage)
    }
}

/// Ephemeral construction capacities. Helpers borrow this one array instead of copying a
/// large plan through nested calls or retaining its derived envelopes in Scene edit state.
struct DeformationStorageBuilder {
    capacities: [CapacityPlan; 17],
}

impl DeformationStorageBuilder {
    fn allocate<E>(&self) -> BuildResult<DeformationStorage, E> {
        Ok(DeformationStorage {
            skins: allocate(self.capacities[SKINS])?,
            logical_morphs: allocate(self.capacities[LOGICAL_MORPHS])?,
            morph_definitions: allocate(self.capacities[MORPH_DEFINITIONS])?,
            primitives: allocate(self.capacities[PRIMITIVES])?,
            morph_targets: allocate(self.capacities[MORPH_TARGETS])?,
            morph_deltas: allocate(self.capacities[MORPH_DELTAS])?,
            mesh_instances: allocate(self.capacities[MESH_INSTANCES])?,
            joints: allocate(self.capacities[JOINTS])?,
            inverse_bind_matrices: allocate(self.capacities[INVERSE_BIND_MATRICES])?,
            names: allocate(self.capacities[NAMES])?,
            influence_vertex_offsets: allocate(self.capacities[INFLUENCE_VERTEX_OFFSETS])?,
            influence_joints: allocate(self.capacities[INFLUENCE_JOINTS])?,
            influence_weights: allocate(self.capacities[INFLUENCE_WEIGHTS])?,
            influence_source_orders: allocate(self.capacities[INFLUENCE_SOURCE_ORDERS])?,
            morph_weights: allocate(self.capacities[MORPH_WEIGHTS])?,
            morph_values: allocate(self.capacities[MORPH_VALUES])?,
            morph_sparse_indices: allocate(self.capacities[MORPH_SPARSE_INDICES])?,
        })
    }

    fn append_skin<C: TypedStorageBuildControl>(
        &self,
        storage: &mut DeformationStorage,
        skin: SkinAuthoring<'_>,
        node_mapping: &[u32],
        control: &mut C,
    ) -> BuildResult<(), C::Error> {
        if skin.joints().len() != skin.inverse_bind_matrices().len() {
            return Err(TypedStorageBuildError::Invariant);
        }
        let name = append_name(
            &mut storage.names,
            self.capacities[NAMES],
            skin.name(),
            control,
        )?;
        let joint_start = storage.joints.len();
        for &joint in skin.joints() {
            let canonical = *node_mapping
                .get(joint.index() as usize)
                .filter(|&&index| index != 0)
                .ok_or(TypedStorageBuildError::Invariant)?;
            push(
                &mut storage.joints,
                self.capacities[JOINTS],
                StoredNodeIndex(canonical),
                control,
            )?;
        }
        let inverse_bind_matrices = append(
            &mut storage.inverse_bind_matrices,
            self.capacities[INVERSE_BIND_MATRICES],
            skin.inverse_bind_matrices(),
            control,
        )?;
        let skeleton_root = match skin.skeleton_root() {
            Presence::Absent => None,
            Presence::Present(root) => Some(StoredNodeIndex(
                *node_mapping
                    .get(root.index() as usize)
                    .filter(|&&index| index != 0)
                    .ok_or(TypedStorageBuildError::Invariant)?,
            )),
        };
        push(
            &mut storage.skins,
            self.capacities[SKINS],
            StoredSkin {
                name,
                joints: TableRange::new(joint_start, skin.joints().len()),
                inverse_bind_matrices,
                skeleton_root,
                matrix_spaces: skin.matrix_spaces(),
            },
            control,
        )
    }

    fn append_influences<C: TypedStorageBuildControl>(
        &self,
        storage: &mut DeformationStorage,
        rows: InfluenceRowsAuthoring<'_>,
        control: &mut C,
    ) -> BuildResult<StoredInfluenceRows, C::Error> {
        let vertex_offsets = append(
            &mut storage.influence_vertex_offsets,
            self.capacities[INFLUENCE_VERTEX_OFFSETS],
            rows.vertex_offsets(),
            control,
        )?;
        let start = storage.influence_joints.len();
        if start != storage.influence_weights.len()
            || start != storage.influence_source_orders.len()
        {
            return Err(TypedStorageBuildError::Invariant);
        }
        for &influence in rows.influences() {
            push(
                &mut storage.influence_joints,
                self.capacities[INFLUENCE_JOINTS],
                influence.joint(),
                control,
            )?;
            push(
                &mut storage.influence_weights,
                self.capacities[INFLUENCE_WEIGHTS],
                influence.weight(),
                control,
            )?;
            push(
                &mut storage.influence_source_orders,
                self.capacities[INFLUENCE_SOURCE_ORDERS],
                influence.source_order(),
                control,
            )?;
        }
        Ok(StoredInfluenceRows {
            vertex_offsets,
            influences: TableRange::new(start, rows.influences().len()),
        })
    }

    fn append_morph_payload<C: TypedStorageBuildControl>(
        &self,
        storage: &mut DeformationStorage,
        payload: MorphDeltaPayloadAuthoring<'_>,
        control: &mut C,
    ) -> BuildResult<StoredMorphDeltaPayload, C::Error> {
        Ok(match payload {
            MorphDeltaPayloadAuthoring::Dense { values } => StoredMorphDeltaPayload::Dense(append(
                &mut storage.morph_values,
                self.capacities[MORPH_VALUES],
                values,
                control,
            )?),
            MorphDeltaPayloadAuthoring::Sparse {
                logical_count,
                indices,
                values,
            } => StoredMorphDeltaPayload::Sparse {
                logical_count,
                indices: append(
                    &mut storage.morph_sparse_indices,
                    self.capacities[MORPH_SPARSE_INDICES],
                    indices,
                    control,
                )?,
                values: append(
                    &mut storage.morph_values,
                    self.capacities[MORPH_VALUES],
                    values,
                    control,
                )?,
            },
        })
    }

    fn finish<E>(&self, storage: &DeformationStorage) -> BuildResult<(), E> {
        finish(&storage.skins, self.capacities[SKINS])?;
        finish(&storage.logical_morphs, self.capacities[LOGICAL_MORPHS])?;
        finish(
            &storage.morph_definitions,
            self.capacities[MORPH_DEFINITIONS],
        )?;
        finish(&storage.primitives, self.capacities[PRIMITIVES])?;
        finish(&storage.morph_targets, self.capacities[MORPH_TARGETS])?;
        finish(&storage.morph_deltas, self.capacities[MORPH_DELTAS])?;
        finish(&storage.mesh_instances, self.capacities[MESH_INSTANCES])?;
        finish(&storage.joints, self.capacities[JOINTS])?;
        finish(
            &storage.inverse_bind_matrices,
            self.capacities[INVERSE_BIND_MATRICES],
        )?;
        finish(&storage.names, self.capacities[NAMES])?;
        finish(
            &storage.influence_vertex_offsets,
            self.capacities[INFLUENCE_VERTEX_OFFSETS],
        )?;
        finish(&storage.influence_joints, self.capacities[INFLUENCE_JOINTS])?;
        finish(
            &storage.influence_weights,
            self.capacities[INFLUENCE_WEIGHTS],
        )?;
        finish(
            &storage.influence_source_orders,
            self.capacities[INFLUENCE_SOURCE_ORDERS],
        )?;
        finish(&storage.morph_weights, self.capacities[MORPH_WEIGHTS])?;
        finish(&storage.morph_values, self.capacities[MORPH_VALUES])?;
        finish(
            &storage.morph_sparse_indices,
            self.capacities[MORPH_SPARSE_INDICES],
        )
    }
}

fn lengths<const N: usize>(counts: [usize; N]) -> Result<[u64; N], StorePlanError> {
    let mut lengths = [0; N];
    for (index, count) in counts.into_iter().enumerate() {
        lengths[index] = u64::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?;
    }
    Ok(lengths)
}
