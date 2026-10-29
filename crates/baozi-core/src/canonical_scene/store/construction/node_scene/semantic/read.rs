use super::*;

impl<'scene> NodeSceneSemanticView<'scene> {
    pub(in crate::canonical_scene::store) fn node_at(
        self,
        source_index: usize,
    ) -> NodeSceneSemanticResult<NodeSceneSemanticNode<'scene>> {
        if source_index >= self.proof.source_node_count() {
            return Err(invariant("node row is outside the validated Scene"));
        }
        let row = source_index + 1;
        let name_offset = read_u32_as_usize(
            self.nodes,
            row,
            node_scene_node_field::NAME_OFFSET,
            "node name offset is unreadable",
        )?;
        let mut cursor = self.into_cursor()?;
        cursor.roots = None;
        cursor.next_node_index = source_index;
        cursor.next_node_name_offset = name_offset;
        cursor
            .next_node()?
            .ok_or_else(|| invariant("node row is absent"))
    }

    pub(in crate::canonical_scene::store) fn roots(
        self,
    ) -> NodeSceneSemanticResult<NodeSceneChildCursor<'scene>> {
        self.into_cursor()?.take_roots()
    }
}
