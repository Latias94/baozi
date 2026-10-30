//! Constant-time row selection without constructing or resetting an exhaustive cursor.

use super::*;

impl<'scene> NodeSceneDeformationTables<'scene> {
    pub(in crate::canonical_scene::store) fn skin_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneDeformationSkin<'scene>> {
        let skin = self
            .storage
            .skins
            .get(row)
            .ok_or_else(|| invariant("skin row is outside its proof-qualified table"))?;
        record_row_read();
        let joints = borrow_range(skin.joints, &self.storage.joints)?;
        let matrices = borrow_range(
            skin.inverse_bind_matrices,
            &self.storage.inverse_bind_matrices,
        )?;
        if joints.len() != matrices.len() {
            return Err(invariant(
                "skin palette and inverse-bind matrix counts disagree",
            ));
        }
        if let Some(correction) = skin.matrix_spaces.correction() {
            let matrix = correction.matrix();
            if !matrix.is_finite() || !matrix.is_affine() {
                return Err(invariant(
                    "validated skin correction matrix is not finite affine data",
                ));
            }
        }
        Ok(NodeSceneDeformationSkin {
            source_index: row,
            stored: skin,
            name: borrow_name(skin.name, &self.storage.names)?,
            skeleton_root: skin
                .skeleton_root
                .map(|index| source_node(index, self.source_node_count))
                .transpose()?,
            joints: Some(NodeSceneDeformationJointCursor::new(
                joints,
                self.source_node_count,
            )),
            inverse_bind_matrices: Some(NodeSceneDeformationMatrixCursor::new(matrices)),
        })
    }

    pub(in crate::canonical_scene::store) fn logical_morph_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneDeformationLogicalMorph<'scene>> {
        let logical =
            self.storage.logical_morphs.get(row).ok_or_else(|| {
                invariant("logical-morph row is outside its proof-qualified table")
            })?;
        record_row_read();
        let definitions = borrow_range(logical.definitions, &self.storage.morph_definitions)?;
        let defaults = borrow_range(logical.default_weights, &self.storage.morph_weights)?;
        if definitions.len() != defaults.len() {
            return Err(invariant(
                "logical-morph definitions and default weights disagree",
            ));
        }
        Ok(NodeSceneDeformationLogicalMorph {
            source_index: row,
            definition_offset: logical.definitions.start(),
            definition_count: definitions.len(),
            default_weights: Some(NodeSceneDeformationF32Cursor::new(defaults)),
        })
    }

    pub(in crate::canonical_scene::store) fn morph_definition_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneDeformationMorphDefinition<'scene>> {
        let definition = self.storage.morph_definitions.get(row).ok_or_else(|| {
            invariant("morph-definition row is outside its proof-qualified table")
        })?;
        record_row_read();
        Ok(NodeSceneDeformationMorphDefinition {
            source_index: row,
            logical_mesh: definition.logical_mesh.0 as usize,
            target_ordinal: definition.target_ordinal as usize,
            stable_identity: definition.stable_identity.value(),
            name: borrow_name(definition.name, &self.storage.names)?,
        })
    }

    pub(in crate::canonical_scene::store) fn primitive_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneDeformationPrimitive<'scene>> {
        let primitive = self.storage.primitives.get(row).ok_or_else(|| {
            invariant("deformation primitive row is outside its proof-qualified table")
        })?;
        record_row_read();
        let targets = borrow_range(primitive.morph_targets, &self.storage.morph_targets)?;
        let influences = primitive
            .influences
            .map(|rows| {
                let range = rows
                    .influences
                    .checked(self.storage.influence_joints.len())
                    .ok_or_else(|| invariant("validated influence range is unavailable"))?;
                NodeSceneDeformationInfluenceRowsCursor::try_new(
                    borrow_range(rows.vertex_offsets, &self.storage.influence_vertex_offsets)?,
                    &self.storage.influence_joints[range.clone()],
                    self.storage
                        .influence_weights
                        .get(range.clone())
                        .ok_or_else(|| {
                            invariant("validated influence-weight range is unavailable")
                        })?,
                    self.storage
                        .influence_source_orders
                        .get(range)
                        .ok_or_else(|| {
                            invariant("validated influence source-order range is unavailable")
                        })?,
                )
            })
            .transpose()?;
        Ok(NodeSceneDeformationPrimitive {
            source_index: row,
            influences,
            morph_target_offset: primitive.morph_targets.start(),
            morph_target_count: targets.len(),
        })
    }

    pub(in crate::canonical_scene::store) fn morph_target_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneDeformationMorphTarget> {
        let target =
            self.storage.morph_targets.get(row).ok_or_else(|| {
                invariant("morph-target row is outside its proof-qualified table")
            })?;
        record_row_read();
        let deltas = borrow_range(target.deltas, &self.storage.morph_deltas)?;
        Ok(NodeSceneDeformationMorphTarget {
            source_index: row,
            primitive: target.primitive as usize,
            target_ordinal: target.target_ordinal as usize,
            delta_offset: target.deltas.start(),
            delta_count: deltas.len(),
        })
    }

    pub(in crate::canonical_scene::store) fn morph_delta_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneDeformationMorphDelta<'scene>> {
        let delta = self
            .storage
            .morph_deltas
            .get(row)
            .ok_or_else(|| invariant("morph-delta row is outside its proof-qualified table"))?;
        record_row_read();
        let payload = match delta.payload {
            StoredMorphDeltaPayload::Dense(range) => {
                let values = NodeSceneDeformationVec3Cursor::try_new(borrow_range(
                    range,
                    &self.storage.morph_values,
                )?)?;
                NodeSceneDeformationMorphPayload::Dense {
                    logical_count: values.declared_count(),
                    values,
                }
            }
            StoredMorphDeltaPayload::Sparse {
                logical_count,
                indices,
                values,
            } => {
                let logical_count = logical_count as usize;
                NodeSceneDeformationMorphPayload::Sparse {
                    logical_count,
                    values: NodeSceneDeformationSparseDeltaCursor::try_new(
                        logical_count,
                        borrow_range(indices, &self.storage.morph_sparse_indices)?,
                        borrow_range(values, &self.storage.morph_values)?,
                    )?,
                }
            }
        };
        Ok(NodeSceneDeformationMorphDelta {
            source_index: row,
            morph_target: delta.target.0 as usize,
            semantic: delta.semantic,
            payload,
        })
    }

    pub(in crate::canonical_scene::store) fn mesh_instance_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneDeformationMeshInstance<'scene>> {
        let instance = self.storage.mesh_instances.get(row).ok_or_else(|| {
            invariant("deformation instance row is outside its proof-qualified table")
        })?;
        record_row_read();
        Ok(NodeSceneDeformationMeshInstance {
            source_index: row,
            skin: instance.skin.map(|index| index.0 as usize),
            morph_weights: instance
                .morph_weights
                .map(|range| {
                    borrow_range(range, &self.storage.morph_weights)
                        .map(NodeSceneDeformationF32Cursor::new)
                })
                .transpose()?,
        })
    }
}
