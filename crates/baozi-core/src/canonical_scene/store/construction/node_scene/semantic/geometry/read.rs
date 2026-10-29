use super::*;

// Random reads use the same decoders as the exhaustive evidence traversal. The Scene is
// immutable and centrally validated, so preceding rows need not be revisited.
impl<'scene> NodeSceneGeometryTables<'scene> {
    pub(in crate::canonical_scene::store) fn logical_mesh_at(
        self,
        proof: NodeSceneValidationProof,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneGeometryLogicalMesh<'scene>> {
        let table = self
            .logical_meshes
            .ok_or_else(|| invariant("logical-mesh table is absent"))?;
        let offset = read_u32_as_usize(
            table,
            row,
            geometry_logical_mesh_field::NAME_OFFSET,
            "logical-mesh name offset is unreadable",
        )?;
        let mut cursor = self.into_cursor(proof);
        cursor.next_logical_mesh = row;
        cursor.next_logical_mesh_name_offset = offset;
        cursor
            .next_logical_mesh()?
            .ok_or_else(|| invariant("logical-mesh row is absent"))
    }

    pub(in crate::canonical_scene::store) fn primitive_at(
        self,
        proof: NodeSceneValidationProof,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneGeometryPrimitive<'scene>> {
        let mut cursor = self.into_cursor(proof);
        cursor.next_logical_mesh = proof.logical_mesh_count();
        cursor.next_primitive = row;
        cursor
            .next_primitive()?
            .ok_or_else(|| invariant("primitive row is absent"))
    }

    pub(in crate::canonical_scene::store) fn attribute_at(
        self,
        proof: NodeSceneValidationProof,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneGeometryAttribute<'scene>> {
        let table = self
            .attributes
            .ok_or_else(|| invariant("attribute table is absent"))?;
        let offset = read_u32_as_usize(
            table,
            row,
            geometry_attribute_field::SEMANTIC_NAME_OFFSET,
            "attribute semantic-name offset is unreadable",
        )?;
        let mut cursor = self.into_cursor(proof);
        cursor.next_logical_mesh = proof.logical_mesh_count();
        cursor.next_primitive = proof.primitive_count();
        cursor.next_attribute = row;
        cursor.next_attribute_semantic_name_offset = offset;
        cursor
            .next_attribute()?
            .ok_or_else(|| invariant("attribute row is absent"))
    }

    pub(in crate::canonical_scene::store) fn mesh_instance_at(
        self,
        proof: NodeSceneValidationProof,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneGeometryMeshInstance> {
        let mut cursor = self.into_cursor(proof);
        cursor.next_logical_mesh = proof.logical_mesh_count();
        cursor.next_primitive = proof.primitive_count();
        cursor.next_attribute = proof.attribute_count();
        cursor.next_mesh_instance = row;
        cursor
            .next_mesh_instance()?
            .ok_or_else(|| invariant("mesh-instance row is absent"))
    }
}
