//! Ordered traversal and exact coverage of the retained typed tables and payload ranges.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DeformationCursorPhase {
    Skins,
    LogicalMorphs,
    MorphDefinitions,
    Primitives,
    MorphTargets,
    MorphDeltas,
    MeshInstances,
}

#[must_use = "the deformation cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneDeformationCursor<'scene> {
    tables: NodeSceneDeformationTables<'scene>,
    phase: DeformationCursorPhase,
    next_skin: usize,
    next_logical_morph: usize,
    next_morph_definition: usize,
    next_primitive: usize,
    next_morph_target: usize,
    next_morph_delta: usize,
    next_mesh_instance: usize,
    ranges: RangeCursors,
}

impl<'scene> NodeSceneDeformationCursor<'scene> {
    pub(super) fn new(tables: NodeSceneDeformationTables<'scene>) -> Self {
        Self {
            tables,
            phase: DeformationCursorPhase::Skins,
            next_skin: 0,
            next_logical_morph: 0,
            next_morph_definition: 0,
            next_primitive: 0,
            next_morph_target: 0,
            next_morph_delta: 0,
            next_mesh_instance: 0,
            ranges: RangeCursors::default(),
        }
    }

    pub(in crate::canonical_scene::store) fn skin_count(&self) -> usize {
        self.tables.storage.skins.len()
    }

    pub(in crate::canonical_scene::store) fn logical_morph_count(&self) -> usize {
        self.tables.storage.logical_morphs.len()
    }

    pub(in crate::canonical_scene::store) fn morph_definition_count(&self) -> usize {
        self.tables.storage.morph_definitions.len()
    }

    pub(in crate::canonical_scene::store) fn primitive_count(&self) -> usize {
        self.tables.storage.primitives.len()
    }

    pub(in crate::canonical_scene::store) fn morph_target_count(&self) -> usize {
        self.tables.storage.morph_targets.len()
    }

    pub(in crate::canonical_scene::store) fn morph_delta_count(&self) -> usize {
        self.tables.storage.morph_deltas.len()
    }

    pub(in crate::canonical_scene::store) fn mesh_instance_count(&self) -> usize {
        self.tables.storage.mesh_instances.len()
    }

    pub(in crate::canonical_scene::store) fn next_skin(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationSkin<'scene>>> {
        self.enter_phase(DeformationCursorPhase::Skins)?;
        let Some(row) = next_row(self.next_skin, self.skin_count())? else {
            return Ok(None);
        };
        let view = self.tables.skin_at(row)?;
        consume(view.stored.joints, &mut self.ranges.joints)?;
        consume(
            view.stored.inverse_bind_matrices,
            &mut self.ranges.inverse_binds,
        )?;
        consume_name(view.stored.name, &mut self.ranges.names)?;
        self.next_skin += 1;
        Ok(Some(view))
    }

    pub(in crate::canonical_scene::store) fn next_logical_morph(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationLogicalMorph<'scene>>> {
        self.enter_phase(DeformationCursorPhase::LogicalMorphs)?;
        let Some(row) = next_row(self.next_logical_morph, self.logical_morph_count())? else {
            return Ok(None);
        };
        let view = self.tables.logical_morph_at(row)?;
        let logical = &self.tables.storage.logical_morphs[row];
        consume(logical.definitions, &mut self.ranges.definitions)?;
        consume(logical.default_weights, &mut self.ranges.weights)?;
        self.next_logical_morph += 1;
        Ok(Some(view))
    }

    pub(in crate::canonical_scene::store) fn next_morph_definition(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationMorphDefinition<'scene>>> {
        self.enter_phase(DeformationCursorPhase::MorphDefinitions)?;
        let Some(row) = next_row(self.next_morph_definition, self.morph_definition_count())? else {
            return Ok(None);
        };
        let view = self.tables.morph_definition_at(row)?;
        let owner = self
            .tables
            .storage
            .logical_morphs
            .get(view.logical_mesh())
            .ok_or_else(|| invariant("validated morph-definition owner is absent"))?;
        if view.target_ordinal() >= owner.definitions.len()
            || owner.definitions.start().checked_add(view.target_ordinal()) != Some(row)
            || view.stable_identity() as usize >= owner.definitions.len()
        {
            return Err(invariant(
                "validated morph-definition ownership is inconsistent",
            ));
        }
        consume_name(
            self.tables.storage.morph_definitions[row].name,
            &mut self.ranges.names,
        )?;
        self.next_morph_definition += 1;
        Ok(Some(view))
    }

    pub(in crate::canonical_scene::store) fn next_primitive(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationPrimitive<'scene>>> {
        self.enter_phase(DeformationCursorPhase::Primitives)?;
        let Some(row) = next_row(self.next_primitive, self.primitive_count())? else {
            return Ok(None);
        };
        let view = self.tables.primitive_at(row)?;
        let primitive = &self.tables.storage.primitives[row];
        consume(primitive.morph_targets, &mut self.ranges.targets)?;
        if let Some(influences) = primitive.influences {
            consume(influences.vertex_offsets, &mut self.ranges.vertex_offsets)?;
            consume(influences.influences, &mut self.ranges.influences)?;
        }
        self.next_primitive += 1;
        Ok(Some(view))
    }

    pub(in crate::canonical_scene::store) fn next_morph_target(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationMorphTarget>> {
        self.enter_phase(DeformationCursorPhase::MorphTargets)?;
        let Some(row) = next_row(self.next_morph_target, self.morph_target_count())? else {
            return Ok(None);
        };
        let view = self.tables.morph_target_at(row)?;
        let owner = self
            .tables
            .storage
            .primitives
            .get(view.primitive())
            .ok_or_else(|| invariant("validated morph-target primitive is absent"))?;
        if view.target_ordinal() >= owner.morph_targets.len()
            || owner
                .morph_targets
                .start()
                .checked_add(view.target_ordinal())
                != Some(row)
        {
            return Err(invariant(
                "validated morph-target ownership is inconsistent",
            ));
        }
        consume(
            self.tables.storage.morph_targets[row].deltas,
            &mut self.ranges.deltas,
        )?;
        self.next_morph_target += 1;
        Ok(Some(view))
    }

    pub(in crate::canonical_scene::store) fn next_morph_delta(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationMorphDelta<'scene>>> {
        self.enter_phase(DeformationCursorPhase::MorphDeltas)?;
        let Some(row) = next_row(self.next_morph_delta, self.morph_delta_count())? else {
            return Ok(None);
        };
        let view = self.tables.morph_delta_at(row)?;
        let storage = self.tables.storage;
        let owner = storage
            .morph_targets
            .get(view.morph_target())
            .ok_or_else(|| invariant("validated morph-delta target is absent"))?;
        let owner_range = owner
            .deltas
            .checked(storage.morph_deltas.len())
            .ok_or_else(|| invariant("validated morph-delta owner range is unavailable"))?;
        if !owner_range.contains(&row)
            || (row > owner_range.start
                && semantic_order(storage.morph_deltas[row - 1].semantic)
                    >= semantic_order(view.semantic()))
        {
            return Err(invariant(
                "validated morph-delta ownership or semantic order is inconsistent",
            ));
        }
        match storage.morph_deltas[row].payload {
            StoredMorphDeltaPayload::Dense(values) => consume(values, &mut self.ranges.values)?,
            StoredMorphDeltaPayload::Sparse {
                indices, values, ..
            } => {
                consume(indices, &mut self.ranges.sparse_indices)?;
                consume(values, &mut self.ranges.values)?;
            }
        }
        self.next_morph_delta += 1;
        Ok(Some(view))
    }

    pub(in crate::canonical_scene::store) fn next_mesh_instance(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneDeformationMeshInstance<'scene>>> {
        self.enter_phase(DeformationCursorPhase::MeshInstances)?;
        let Some(row) = next_row(self.next_mesh_instance, self.mesh_instance_count())? else {
            return Ok(None);
        };
        let view = self.tables.mesh_instance_at(row)?;
        if view.skin().is_some_and(|skin| skin >= self.skin_count()) {
            return Err(invariant("validated mesh-instance skin is absent"));
        }
        if let Some(weights) = self.tables.storage.mesh_instances[row].morph_weights {
            consume(weights, &mut self.ranges.weights)?;
        }
        self.next_mesh_instance += 1;
        Ok(Some(view))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        if self.next_skin != self.skin_count()
            || self.next_logical_morph != self.logical_morph_count()
            || self.next_morph_definition != self.morph_definition_count()
            || self.next_primitive != self.primitive_count()
            || self.next_morph_target != self.morph_target_count()
            || self.next_morph_delta != self.morph_delta_count()
            || self.next_mesh_instance != self.mesh_instance_count()
        {
            return Err(invariant(
                "deformation cursor ended before its proof-qualified table counts",
            ));
        }
        if !self.ranges.complete(self.tables.storage) {
            return Err(invariant(
                "deformation relation and payload ranges were not consumed exactly",
            ));
        }
        Ok(())
    }

    fn enter_phase(&mut self, target: DeformationCursorPhase) -> NodeSceneSemanticResult<()> {
        if target < self.phase {
            return Err(invariant(
                "deformation cursor cannot return to an earlier table phase",
            ));
        }
        for (phase, next, count) in [
            (
                DeformationCursorPhase::Skins,
                self.next_skin,
                self.skin_count(),
            ),
            (
                DeformationCursorPhase::LogicalMorphs,
                self.next_logical_morph,
                self.logical_morph_count(),
            ),
            (
                DeformationCursorPhase::MorphDefinitions,
                self.next_morph_definition,
                self.morph_definition_count(),
            ),
            (
                DeformationCursorPhase::Primitives,
                self.next_primitive,
                self.primitive_count(),
            ),
            (
                DeformationCursorPhase::MorphTargets,
                self.next_morph_target,
                self.morph_target_count(),
            ),
            (
                DeformationCursorPhase::MorphDeltas,
                self.next_morph_delta,
                self.morph_delta_count(),
            ),
        ] {
            if phase < target && next != count {
                return Err(invariant(
                    "earlier deformation table must be exhausted before advancing",
                ));
            }
        }
        self.phase = target;
        Ok(())
    }
}

/// Names span skins and definitions; weights span defaults and present instance overrides.
/// A shared cursor for each pool also accounts for authored present-but-empty ranges.
#[derive(Default)]
struct RangeCursors {
    joints: usize,
    inverse_binds: usize,
    names: usize,
    definitions: usize,
    weights: usize,
    vertex_offsets: usize,
    influences: usize,
    targets: usize,
    deltas: usize,
    values: usize,
    sparse_indices: usize,
}

impl RangeCursors {
    fn complete(&self, storage: &DeformationStorage) -> bool {
        self.joints == storage.joints.len()
            && self.inverse_binds == storage.inverse_bind_matrices.len()
            && self.names == storage.names.len()
            && self.definitions == storage.morph_definitions.len()
            && self.weights == storage.morph_weights.len()
            && self.vertex_offsets == storage.influence_vertex_offsets.len()
            && self.influences == storage.influence_joints.len()
            && self.influences == storage.influence_weights.len()
            && self.influences == storage.influence_source_orders.len()
            && self.targets == storage.morph_targets.len()
            && self.deltas == storage.morph_deltas.len()
            && self.values == storage.morph_values.len()
            && self.sparse_indices == storage.morph_sparse_indices.len()
    }
}

fn consume<T>(range: TableRange<T>, cursor: &mut usize) -> NodeSceneSemanticResult<()> {
    if range.start() != *cursor {
        return Err(invariant("validated deformation ranges are not contiguous"));
    }
    *cursor = cursor
        .checked_add(range.len())
        .ok_or_else(|| invariant("deformation range cursor overflows"))?;
    Ok(())
}

fn consume_name(name: Option<TextRange>, cursor: &mut usize) -> NodeSceneSemanticResult<()> {
    if let Some(name) = name {
        consume(name.0, cursor)?;
    }
    Ok(())
}

fn next_row(next: usize, count: usize) -> NodeSceneSemanticResult<Option<usize>> {
    if next > count {
        return Err(invariant(
            "deformation cursor advanced beyond its proof-qualified count",
        ));
    }
    Ok((next < count).then_some(next))
}

fn semantic_order(semantic: MorphDeltaSemantic) -> u8 {
    match semantic {
        MorphDeltaSemantic::Position => 0,
        MorphDeltaSemantic::Normal => 1,
        MorphDeltaSemantic::Tangent => 2,
    }
}
