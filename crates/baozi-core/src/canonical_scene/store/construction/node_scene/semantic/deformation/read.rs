//! Direct deformation rows sharing the exhaustive semantic decoders.

use super::*;

impl<'scene> NodeSceneDeformationTables<'scene> {
    pub(in crate::canonical_scene::store) fn skin_at(
        self,
        proof: NodeSceneValidationProof,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneDeformationSkin<'scene>> {
        let table = require_row(self.skins, row, proof.skin_count())?;
        let mut cursor = self.into_row_cursor(proof);
        cursor.next_skin = row;
        cursor.joint_cursor = offset(table, row, deformation_skin_field::JOINT_OFFSET)?;
        cursor.inverse_bind_cursor =
            offset(table, row, deformation_skin_field::INVERSE_BIND_OFFSET)?;
        cursor.correction_cursor =
            offset(table, row, deformation_skin_field::CORRECTION_MATRIX_OFFSET)?;
        cursor.skin_name_cursor = offset(table, row, deformation_skin_field::NAME_OFFSET)?;
        cursor
            .next_skin()?
            .ok_or_else(|| invariant("validated skin row is absent"))
    }

    pub(in crate::canonical_scene::store) fn logical_morph_at(
        self,
        proof: NodeSceneValidationProof,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneDeformationLogicalMorph<'scene>> {
        let table = require_row(self.logical_morphs, row, proof.logical_morph_count())?;
        let mut cursor = self.into_row_cursor(proof);
        cursor.next_logical_morph = row;
        cursor.definition_cursor = offset(
            table,
            row,
            deformation_logical_morph_field::DEFINITION_OFFSET,
        )?;
        cursor.morph_default_cursor = offset(
            table,
            row,
            deformation_logical_morph_field::DEFAULT_WEIGHT_OFFSET,
        )?;
        cursor
            .next_logical_morph()?
            .ok_or_else(|| invariant("validated logical-morph row is absent"))
    }

    pub(in crate::canonical_scene::store) fn morph_definition_at(
        self,
        proof: NodeSceneValidationProof,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneDeformationMorphDefinition<'scene>> {
        let table = require_row(self.morph_definitions, row, proof.morph_definition_count())?;
        let mut cursor = self.into_row_cursor(proof);
        cursor.next_morph_definition = row;
        cursor.morph_name_cursor =
            offset(table, row, deformation_morph_definition_field::NAME_OFFSET)?;
        cursor
            .next_morph_definition()?
            .ok_or_else(|| invariant("validated morph-definition row is absent"))
    }

    pub(in crate::canonical_scene::store) fn primitive_at(
        self,
        proof: NodeSceneValidationProof,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneDeformationPrimitive<'scene>> {
        let table = require_row(self.primitives, row, proof.primitive_deformation_count())?;
        let mut cursor = self.into_row_cursor(proof);
        cursor.next_primitive = row;
        cursor.influence_row_offset_cursor =
            offset(table, row, deformation_primitive_field::ROW_OFFSET_OFFSET)?;
        cursor.influence_cursor =
            offset(table, row, deformation_primitive_field::INFLUENCE_OFFSET)?;
        cursor.target_cursor =
            offset(table, row, deformation_primitive_field::MORPH_TARGET_OFFSET)?;
        cursor
            .next_primitive()?
            .ok_or_else(|| invariant("validated primitive-deformation row is absent"))
    }

    pub(in crate::canonical_scene::store) fn morph_target_at(
        self,
        proof: NodeSceneValidationProof,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneDeformationMorphTarget> {
        let table = require_row(self.morph_targets, row, proof.morph_target_count())?;
        let mut cursor = self.into_row_cursor(proof);
        cursor.next_morph_target = row;
        cursor.delta_cursor = offset(table, row, deformation_morph_target_field::DELTA_OFFSET)?;
        cursor
            .next_morph_target()?
            .ok_or_else(|| invariant("validated morph-target row is absent"))
    }

    pub(in crate::canonical_scene::store) fn morph_delta_at(
        self,
        proof: NodeSceneValidationProof,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneDeformationMorphDelta<'scene>> {
        let table = require_row(self.morph_deltas, row, proof.morph_delta_count())?;
        let mut cursor = self.into_row_cursor(proof);
        cursor.next_morph_delta = row;
        cursor.morph_value_cursor =
            offset(table, row, deformation_morph_delta_field::VALUE_OFFSET)?;
        cursor.sparse_index_cursor = offset(
            table,
            row,
            deformation_morph_delta_field::SPARSE_INDEX_OFFSET,
        )?;
        cursor
            .next_morph_delta()?
            .ok_or_else(|| invariant("validated morph-delta row is absent"))
    }

    pub(in crate::canonical_scene::store) fn mesh_instance_at(
        self,
        proof: NodeSceneValidationProof,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneDeformationMeshInstance<'scene>> {
        let table = require_row(self.mesh_instances, row, proof.mesh_instance_count())?;
        let mut cursor = self.into_row_cursor(proof);
        cursor.next_mesh_instance = row;
        cursor.instance_weight_cursor = offset(
            table,
            row,
            deformation_mesh_instance_field::MORPH_WEIGHT_OFFSET,
        )?;
        cursor
            .next_mesh_instance()?
            .ok_or_else(|| invariant("validated mesh-instance deformation row is absent"))
    }

    fn into_row_cursor(
        self,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneDeformationCursor<'scene> {
        // Central validation has established every relation and payload. Seed only the selected
        // row's recorded offsets and mark unrelated phases complete; never walk preceding rows.
        let mut cursor = self.into_cursor(proof);
        cursor.validate_payloads = false;
        cursor.next_skin = proof.skin_count();
        cursor.next_logical_morph = proof.logical_morph_count();
        cursor.next_morph_definition = proof.morph_definition_count();
        cursor.next_primitive = proof.primitive_deformation_count();
        cursor.next_morph_target = proof.morph_target_count();
        cursor.next_morph_delta = proof.morph_delta_count();
        cursor.next_mesh_instance = proof.mesh_instance_count();
        cursor
    }
}

fn require_row(
    table: Option<CanonicalRecordView<'_>>,
    row: usize,
    count: usize,
) -> NodeSceneSemanticResult<CanonicalRecordView<'_>> {
    if row >= count {
        return Err(invariant(
            "deformation row is outside its proof-qualified table",
        ));
    }
    table.ok_or_else(|| invariant("validated deformation table view is absent"))
}

fn offset(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
) -> NodeSceneSemanticResult<usize> {
    read_u32_as_usize(
        table,
        row,
        field,
        "validated deformation payload offset is unreadable",
    )
}

impl<'scene> NodeSceneDeformationJointCursor<'scene> {
    pub(in crate::canonical_scene::store) const fn into_slice(self) -> &'scene [u32] {
        self.canonical_nodes
    }
}

impl<'scene> NodeSceneDeformationMatrixCursor<'scene> {
    pub(in crate::canonical_scene::store) fn into_matrices(
        self,
    ) -> impl ExactSizeIterator<Item = NodeSceneSemanticResult<Mat4>>
    + DoubleEndedIterator
    + Clone
    + 'scene {
        self.scalars
            .as_chunks::<MATRIX_SCALAR_COUNT>()
            .0
            .iter()
            .map(|values| matrix_from_slice(values))
    }
}

impl<'scene> NodeSceneDeformationVec3Cursor<'scene> {
    pub(in crate::canonical_scene::store) const fn into_slice(self) -> &'scene [f32] {
        self.scalars
    }
}

impl<'scene> NodeSceneDeformationSparseDeltaCursor<'scene> {
    pub(in crate::canonical_scene::store) const fn into_slices(
        self,
    ) -> (&'scene [u32], &'scene [f32]) {
        (self.indices, self.values.into_slice())
    }
}

impl<'scene> NodeSceneDeformationInfluenceRowsCursor<'scene> {
    pub(in crate::canonical_scene::store) const fn into_slices(
        self,
    ) -> (&'scene [u32], &'scene [u32], &'scene [f32], &'scene [u32]) {
        (self.offsets, self.joints, self.weights, self.source_orders)
    }
}

impl<'scene, T: StoredScalar> NodeSceneDeformationLeafCursor<'scene, T> {
    pub(in crate::canonical_scene::store) const fn into_slice(self) -> &'scene [T] {
        self.values
    }
}
