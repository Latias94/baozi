//! Central validation over the actual deformation rows and their borrowed payloads.
//!
//! Only palette maxima and uniqueness marks use mutable algorithm scratch. No semantic row is
//! decoded, copied to a descriptor table, or reconstructed from a record-store member.
//! Skin bind and correction matrices require finite affine values. Singular matrices, shear,
//! reflections, and authored signed zeros remain valid and are never decomposed or rewritten.

use crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl;
use crate::canonical_scene::store::typed_storage::{GeometryStorage, NodeStorage, TableRange};
use crate::deform::{MorphDeltaSemantic, SkinMatrixSpace, SkinMatrixSpaces};
use crate::entity::{MAX_ENTITY_NAME_BYTES, validate_entity_name_str};

use super::*;

#[cfg(test)]
mod tests;

/// Initialized element counts in the same row/payload order as DeformationStoragePlan.
/// The three influence columns have one common count; inverse binds count whole Mat4 values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct DeformationMeteredCounts {
    pub(in crate::canonical_scene::store) skin_count: u32,
    pub(in crate::canonical_scene::store) logical_morph_count: u32,
    pub(in crate::canonical_scene::store) morph_definition_count: u32,
    pub(in crate::canonical_scene::store) primitive_count: u32,
    pub(in crate::canonical_scene::store) morph_target_count: u32,
    pub(in crate::canonical_scene::store) morph_delta_count: u32,
    pub(in crate::canonical_scene::store) mesh_instance_count: u32,
    pub(in crate::canonical_scene::store) joint_node_count: u64,
    pub(in crate::canonical_scene::store) inverse_bind_matrix_count: u64,
    pub(in crate::canonical_scene::store) name_bytes: u64,
    pub(in crate::canonical_scene::store) influence_row_offset_count: u64,
    pub(in crate::canonical_scene::store) influence_count: u64,
    pub(in crate::canonical_scene::store) morph_weight_count: u64,
    pub(in crate::canonical_scene::store) morph_value_count: u64,
    pub(in crate::canonical_scene::store) morph_sparse_index_count: u64,
}

impl DeformationMeteredCounts {
    pub(in crate::canonical_scene::store) fn try_new(
        rows: [u64; 7],
        payloads: [u64; 10],
    ) -> Option<Self> {
        if payloads[0] != payloads[1]
            || payloads[4] != payloads[5]
            || payloads[4] != payloads[6]
            || !payloads[8].is_multiple_of(3)
        {
            return None;
        }
        for count in payloads {
            usize::try_from(count).ok()?;
        }
        Some(Self {
            skin_count: u32::try_from(rows[0]).ok()?,
            logical_morph_count: u32::try_from(rows[1]).ok()?,
            morph_definition_count: u32::try_from(rows[2]).ok()?,
            primitive_count: u32::try_from(rows[3]).ok()?,
            morph_target_count: u32::try_from(rows[4]).ok()?,
            morph_delta_count: u32::try_from(rows[5]).ok()?,
            mesh_instance_count: u32::try_from(rows[6]).ok()?,
            joint_node_count: payloads[0],
            inverse_bind_matrix_count: payloads[1],
            name_bytes: payloads[2],
            influence_row_offset_count: payloads[3],
            influence_count: payloads[4],
            morph_weight_count: payloads[7],
            morph_value_count: payloads[8],
            morph_sparse_index_count: payloads[9],
        })
    }

    /// Reads only the seventeen Vec lengths; this is not a semantic planning pass.
    pub(in crate::canonical_scene::store) fn for_storage(
        storage: &DeformationStorage,
    ) -> Option<Self> {
        Self::try_new(
            [
                storage.skins.len() as u64,
                storage.logical_morphs.len() as u64,
                storage.morph_definitions.len() as u64,
                storage.primitives.len() as u64,
                storage.morph_targets.len() as u64,
                storage.morph_deltas.len() as u64,
                storage.mesh_instances.len() as u64,
            ],
            [
                storage.joints.len() as u64,
                storage.inverse_bind_matrices.len() as u64,
                storage.names.len() as u64,
                storage.influence_vertex_offsets.len() as u64,
                storage.influence_joints.len() as u64,
                storage.influence_weights.len() as u64,
                storage.influence_source_orders.len() as u64,
                storage.morph_weights.len() as u64,
                storage.morph_values.len() as u64,
                storage.morph_sparse_indices.len() as u64,
            ],
        )
    }

    /// Exact successful validator actions, excluding separately charged scratch initialization.
    ///
    /// Shape and completion each take one action. Every row, name byte, joint, whole inverse-bind
    /// matrix, vertex offset, influence tuple, morph weight/value, and sparse index takes one.
    /// A skin's optional inline correction is checked in its bounded row action.
    pub(in crate::canonical_scene::store) fn validation_work(self) -> Option<u64> {
        [
            2,
            u64::from(self.skin_count),
            u64::from(self.logical_morph_count),
            u64::from(self.morph_definition_count),
            u64::from(self.primitive_count),
            u64::from(self.morph_target_count),
            u64::from(self.morph_delta_count),
            u64::from(self.mesh_instance_count),
            self.joint_node_count,
            self.inverse_bind_matrix_count,
            self.name_bytes,
            self.influence_row_offset_count,
            self.influence_count,
            self.morph_weight_count,
            self.morph_value_count,
            self.morph_sparse_index_count,
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct DeformationValidationScratchRequirements {
    pub(in crate::canonical_scene::store) palette_maxima: usize,
    pub(in crate::canonical_scene::store) uniqueness_marks: usize,
}

impl DeformationValidationScratchRequirements {
    pub(in crate::canonical_scene::store) fn try_new(
        counts: DeformationMeteredCounts,
        maximum_uniqueness_count: usize,
    ) -> Option<Self> {
        let maximum = usize::try_from(
            counts
                .influence_count
                .max(u64::from(counts.morph_definition_count)),
        )
        .ok()?;
        (maximum_uniqueness_count <= maximum).then_some(Self {
            palette_maxima: usize::try_from(counts.logical_morph_count).ok()?,
            uniqueness_marks: maximum_uniqueness_count,
        })
    }

    /// Both buffers are initialized by the caller before validation; charge this work once.
    pub(in crate::canonical_scene::store) fn initialization_work_units(self) -> Option<u64> {
        u64::try_from(self.palette_maxima)
            .ok()?
            .checked_add(u64::try_from(self.uniqueness_marks).ok()?)
    }
}

pub(in crate::canonical_scene::store) struct DeformationValidationScratch<'scratch> {
    pub(in crate::canonical_scene::store) palette_maxima: &'scratch mut [Option<u32>],
    pub(in crate::canonical_scene::store) uniqueness_marks: &'scratch mut [u64],
}

impl<'scratch> DeformationValidationScratch<'scratch> {
    pub(in crate::canonical_scene::store) const fn new(
        palette_maxima: &'scratch mut [Option<u32>],
        uniqueness_marks: &'scratch mut [u64],
    ) -> Self {
        Self {
            palette_maxima,
            uniqueness_marks,
        }
    }

    /// Unit tests use the same separately charged initialization boundary as the central owner.
    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn initialize<C: TypedStorageValidationControl>(
        &mut self,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        for maximum in self.palette_maxima.iter_mut() {
            if control.run_unit(|| *maximum = None)? != Some(()) {
                return Ok(false);
            }
        }
        for mark in self.uniqueness_marks.iter_mut() {
            if control.run_unit(|| *mark = 0)? != Some(()) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct TypedDeformationValidation {
    pub(in crate::canonical_scene::store) maximum_uniqueness_count: usize,
}

impl DeformationStorage {
    pub(in crate::canonical_scene::store) fn validate<C: TypedStorageValidationControl>(
        &self,
        control: &mut C,
        counts: DeformationMeteredCounts,
        nodes: &NodeStorage,
        geometry: &GeometryStorage,
        scratch: &mut DeformationValidationScratch<'_>,
    ) -> Result<Option<TypedDeformationValidation>, C::Error> {
        if control.run_unit(|| {
            DeformationMeteredCounts::for_storage(self) == Some(counts)
                && !nodes.nodes.is_empty()
                && self.logical_morphs.len() == geometry.logical_meshes.len()
                && self.primitives.len() == geometry.primitives.len()
                && self.mesh_instances.len() == geometry.mesh_instances.len()
                && scratch.palette_maxima.len() == self.logical_morphs.len()
        })? != Some(true)
        {
            return Ok(None);
        }
        let DeformationValidationScratch {
            palette_maxima,
            uniqueness_marks,
        } = scratch;
        let mut cursors = Cursors::default();
        let mut uniqueness = Uniqueness::new(uniqueness_marks);
        if !self.validate_skins(control, nodes, &mut cursors)?
            || !self.validate_logical_meshes(
                control,
                geometry,
                &mut cursors,
                palette_maxima,
                &mut uniqueness,
            )?
            || !self.validate_instances(control, nodes, geometry, &mut cursors, palette_maxima)?
            || !self.validate_payloads(control, nodes)?
        {
            return Ok(None);
        }
        run(control, || {
            cursors
                .complete(self)
                .then_some(TypedDeformationValidation {
                    maximum_uniqueness_count: uniqueness.maximum_width,
                })
        })
    }

    fn validate_skins<C: TypedStorageValidationControl>(
        &self,
        control: &mut C,
        nodes: &NodeStorage,
        cursors: &mut Cursors,
    ) -> Result<bool, C::Error> {
        for skin in &self.skins {
            if run(control, || {
                consume(skin.joints, &self.joints, &mut cursors.joints)?;
                consume(
                    skin.inverse_bind_matrices,
                    &self.inverse_bind_matrices,
                    &mut cursors.inverse_binds,
                )?;
                (skin.joints.len() == skin.inverse_bind_matrices.len()).then_some(())?;
                if let Some(root) = skin.skeleton_root {
                    valid_node(root, nodes).then_some(())?;
                }
                valid_matrix_spaces(skin.matrix_spaces).then_some(())?;
                consume_name(skin.name, &self.names, &mut cursors.names)
            })?
            .is_none()
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn validate_logical_meshes<C: TypedStorageValidationControl>(
        &self,
        control: &mut C,
        geometry: &GeometryStorage,
        cursors: &mut Cursors,
        palette_maxima: &mut [Option<u32>],
        uniqueness: &mut Uniqueness<'_>,
    ) -> Result<bool, C::Error> {
        for (mesh_index, logical) in self.logical_morphs.iter().enumerate() {
            let Some((definitions, primitives, marker)) = run(control, || {
                let definitions = consume(
                    logical.definitions,
                    &self.morph_definitions,
                    &mut cursors.definitions,
                )?;
                let defaults = consume(
                    logical.default_weights,
                    &self.morph_weights,
                    &mut cursors.morph_weights,
                )?;
                (definitions.len() == defaults.len()).then_some(())?;
                let mesh = geometry.logical_meshes.get(mesh_index)?;
                (mesh.primitives.start() == cursors.primitives).then_some(())?;
                let range = mesh.primitives.checked(geometry.primitives.len())?;
                let primitives = self.primitives.get(range.clone())?;
                cursors.primitives = range.end;
                let marker = uniqueness.begin_group(definitions.len())?;
                Some((definitions, primitives, marker))
            })?
            else {
                return Ok(false);
            };
            for (ordinal, definition) in definitions.iter().enumerate() {
                if run(control, || {
                    (definition.logical_mesh.0 as usize == mesh_index
                        && definition.target_ordinal as usize == ordinal)
                        .then_some(())?;
                    uniqueness.mark(
                        definition.stable_identity.value(),
                        definitions.len(),
                        marker,
                    )?;
                    consume_name(definition.name, &self.names, &mut cursors.names)
                })?
                .is_none()
                {
                    return Ok(false);
                }
            }
            let primitive_start = geometry.logical_meshes[mesh_index].primitives.start();
            for (ordinal, primitive) in primitives.iter().enumerate() {
                let primitive_index = primitive_start + ordinal;
                let Some((targets, influences)) = run(control, || {
                    let source = geometry.primitives.get(primitive_index)?;
                    let targets = consume(
                        primitive.morph_targets,
                        &self.morph_targets,
                        &mut cursors.targets,
                    )?;
                    (targets.len() == definitions.len()).then_some(())?;
                    if let Some(rows) = primitive.influences {
                        let offsets = consume(
                            rows.vertex_offsets,
                            &self.influence_vertex_offsets,
                            &mut cursors.vertex_offsets,
                        )?;
                        let joints = consume(
                            rows.influences,
                            &self.influence_joints,
                            &mut cursors.influences,
                        )?;
                        (u64::try_from(offsets.len()).ok()? == u64::from(source.vertex_count) + 1)
                            .then_some(())?;
                        Some((targets, Some((offsets, joints, rows.influences.start()))))
                    } else {
                        Some((targets, None))
                    }
                })?
                else {
                    return Ok(false);
                };
                if let Some((offsets, joints, start)) = influences
                    && !self.validate_influences(
                        control,
                        offsets,
                        joints,
                        start,
                        &mut palette_maxima[mesh_index],
                        uniqueness,
                    )?
                {
                    return Ok(false);
                }
                if !self.validate_targets(
                    control,
                    primitive_index,
                    primitive.morph_targets.start(),
                    targets,
                    geometry.primitives[primitive_index].vertex_count,
                    cursors,
                )? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_influences<C: TypedStorageValidationControl>(
        &self,
        control: &mut C,
        offsets: &[u32],
        joints: &[u32],
        start: usize,
        maximum: &mut Option<u32>,
        uniqueness: &mut Uniqueness<'_>,
    ) -> Result<bool, C::Error> {
        let mut previous = 0_usize;
        for (index, &end) in offsets.iter().enumerate() {
            let Some((range, marker)) = run(control, || {
                let end = usize::try_from(end).ok()?;
                (end >= previous
                    && end <= joints.len()
                    && (index != 0 || end == 0)
                    && (index + 1 != offsets.len() || end == joints.len()))
                .then_some(())?;
                let marker = uniqueness.begin_group(end - previous)?;
                let range = previous..end;
                previous = end;
                Some((range, marker))
            })?
            else {
                return Ok(false);
            };
            let width = range.len();
            for local in range {
                if run(control, || {
                    let global = start.checked_add(local)?;
                    let weight = *self.influence_weights.get(global)?;
                    (weight.is_finite() && weight >= 0.0).then_some(())?;
                    let order = *self.influence_source_orders.get(global)?;
                    uniqueness.mark(order, width, marker)?;
                    let joint = *joints.get(local)?;
                    *maximum = Some(maximum.map_or(joint, |value| value.max(joint)));
                    Some(())
                })?
                .is_none()
                {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_targets<C: TypedStorageValidationControl>(
        &self,
        control: &mut C,
        primitive_index: usize,
        target_start: usize,
        targets: &[StoredMorphTarget],
        vertex_count: u32,
        cursors: &mut Cursors,
    ) -> Result<bool, C::Error> {
        for (ordinal, target) in targets.iter().enumerate() {
            let Some(deltas) = run(control, || {
                (target.primitive as usize == primitive_index
                    && target.target_ordinal as usize == ordinal)
                    .then_some(())?;
                consume(target.deltas, &self.morph_deltas, &mut cursors.deltas)
            })?
            else {
                return Ok(false);
            };
            let mut previous_semantic = None;
            for delta in deltas {
                let Some(indices) = run(control, || {
                    let semantic = semantic_ordinal(delta.semantic);
                    (delta.target.0 as usize == target_start.checked_add(ordinal)?
                        && previous_semantic.is_none_or(|previous| previous < semantic))
                    .then_some(())?;
                    previous_semantic = Some(semantic);
                    match delta.payload {
                        StoredMorphDeltaPayload::Dense(range) => {
                            let values = consume(range, &self.morph_values, &mut cursors.values)?;
                            (u64::try_from(values.len()).ok()? == u64::from(vertex_count) * 3)
                                .then_some(&[][..])
                        }
                        StoredMorphDeltaPayload::Sparse {
                            logical_count,
                            indices,
                            values,
                        } => {
                            let indices = consume(
                                indices,
                                &self.morph_sparse_indices,
                                &mut cursors.sparse_indices,
                            )?;
                            let values = consume(values, &self.morph_values, &mut cursors.values)?;
                            (logical_count == vertex_count
                                && indices.len().checked_mul(3) == Some(values.len()))
                            .then_some(indices)
                        }
                    }
                })?
                else {
                    return Ok(false);
                };
                let mut previous = None;
                for &index in indices {
                    if control.run_unit(|| {
                        let valid = index < vertex_count
                            && previous.is_none_or(|previous| previous < index);
                        previous = Some(index);
                        valid
                    })? != Some(true)
                    {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    fn validate_instances<C: TypedStorageValidationControl>(
        &self,
        control: &mut C,
        nodes: &NodeStorage,
        geometry: &GeometryStorage,
        cursors: &mut Cursors,
        palette_maxima: &[Option<u32>],
    ) -> Result<bool, C::Error> {
        for (index, instance) in self.mesh_instances.iter().enumerate() {
            if run(control, || {
                let source = geometry.mesh_instances.get(index)?;
                valid_node(source.owner_node, nodes).then_some(())?;
                let logical_index = usize::try_from(source.logical_mesh.0).ok()?;
                let logical = self.logical_morphs.get(logical_index)?;
                if let Some(skin) = instance.skin {
                    let skin = self.skins.get(usize::try_from(skin.0).ok()?)?;
                    let maximum = *palette_maxima.get(logical_index)?;
                    maximum
                        .is_none_or(|joint| (joint as usize) < skin.joints.len())
                        .then_some(())?;
                }
                if let Some(weights) = instance.morph_weights {
                    let weights =
                        consume(weights, &self.morph_weights, &mut cursors.morph_weights)?;
                    (weights.len() == logical.definitions.len()).then_some(())?;
                }
                Some(())
            })?
            .is_none()
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn validate_payloads<C: TypedStorageValidationControl>(
        &self,
        control: &mut C,
        nodes: &NodeStorage,
    ) -> Result<bool, C::Error> {
        for &joint in &self.joints {
            if control.run_unit(|| valid_node(joint, nodes))? != Some(true) {
                return Ok(false);
            }
        }
        for matrix in &self.inverse_bind_matrices {
            if control.run_unit(|| matrix.is_finite() && matrix.is_affine())? != Some(true) {
                return Ok(false);
            }
        }
        for value in self.morph_weights.iter().chain(&self.morph_values) {
            if control.run_unit(|| value.is_finite())? != Some(true) {
                return Ok(false);
            }
        }
        for &byte in &self.names {
            if control.run_unit(|| byte)? != Some(byte) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[derive(Default)]
struct Cursors {
    joints: usize,
    inverse_binds: usize,
    names: usize,
    definitions: usize,
    primitives: usize,
    targets: usize,
    deltas: usize,
    vertex_offsets: usize,
    influences: usize,
    morph_weights: usize,
    values: usize,
    sparse_indices: usize,
}

impl Cursors {
    fn complete(&self, storage: &DeformationStorage) -> bool {
        self.joints == storage.joints.len()
            && self.inverse_binds == storage.inverse_bind_matrices.len()
            && self.names == storage.names.len()
            && self.definitions == storage.morph_definitions.len()
            && self.primitives == storage.primitives.len()
            && self.targets == storage.morph_targets.len()
            && self.deltas == storage.morph_deltas.len()
            && self.vertex_offsets == storage.influence_vertex_offsets.len()
            && self.influences == storage.influence_joints.len()
            && self.morph_weights == storage.morph_weights.len()
            && self.values == storage.morph_values.len()
            && self.sparse_indices == storage.morph_sparse_indices.len()
    }
}

struct Uniqueness<'scratch> {
    marks: &'scratch mut [u64],
    next_marker: u64,
    maximum_width: usize,
}

impl<'scratch> Uniqueness<'scratch> {
    fn new(marks: &'scratch mut [u64]) -> Self {
        Self {
            marks,
            next_marker: 1,
            maximum_width: 0,
        }
    }

    fn begin_group(&mut self, width: usize) -> Option<u64> {
        (width <= self.marks.len()).then_some(())?;
        let marker = self.next_marker;
        self.next_marker = marker.checked_add(1)?;
        self.maximum_width = self.maximum_width.max(width);
        Some(marker)
    }

    fn mark(&mut self, ordinal: u32, width: usize, marker: u64) -> Option<()> {
        let ordinal = usize::try_from(ordinal).ok()?;
        (ordinal < width).then_some(())?;
        let mark = self.marks.get_mut(ordinal)?;
        (*mark != marker).then_some(())?;
        *mark = marker;
        Some(())
    }
}

fn valid_matrix_spaces(spaces: SkinMatrixSpaces) -> bool {
    spaces.input_position() == SkinMatrixSpace::LogicalMeshLocalBind
        && spaces.inverse_bind_from() == SkinMatrixSpace::LogicalMeshLocalBind
        && spaces.inverse_bind_to() == SkinMatrixSpace::JointLocalBind
        && spaces.current_joint_from() == SkinMatrixSpace::JointLocal
        && spaces.current_joint_to() == SkinMatrixSpace::Scene
        && spaces.output_position() == SkinMatrixSpace::MeshInstanceLocal
        && spaces.correction().is_none_or(|correction| {
            correction.from() == spaces.input_position()
                && correction.to() == spaces.output_position()
                && correction.matrix().is_finite()
                && correction.matrix().is_affine()
        })
}

fn valid_node(index: StoredNodeIndex, nodes: &NodeStorage) -> bool {
    index.0 != 0 && (index.0 as usize) < nodes.nodes.len()
}

fn semantic_ordinal(semantic: MorphDeltaSemantic) -> u8 {
    match semantic {
        MorphDeltaSemantic::Position => 0,
        MorphDeltaSemantic::Normal => 1,
        MorphDeltaSemantic::Tangent => 2,
    }
}

fn consume_name(name: Option<TextRange>, pool: &[u8], cursor: &mut usize) -> Option<()> {
    if let Some(name) = name {
        (name.0.len() <= MAX_ENTITY_NAME_BYTES).then_some(())?;
        let bytes = consume(name.0, pool, cursor)?;
        validate_entity_name_str(std::str::from_utf8(bytes).ok()?).ok()?;
    }
    Some(())
}

fn consume<'storage, T>(
    range: TableRange<T>,
    pool: &'storage [T],
    cursor: &mut usize,
) -> Option<&'storage [T]> {
    (range.start() == *cursor).then_some(())?;
    let values = range.get(pool)?;
    *cursor = cursor.checked_add(values.len())?;
    Some(values)
}

fn run<C: TypedStorageValidationControl, R>(
    control: &mut C,
    action: impl FnOnce() -> Option<R>,
) -> Result<Option<R>, C::Error> {
    Ok(control.run_unit(action)?.flatten())
}
