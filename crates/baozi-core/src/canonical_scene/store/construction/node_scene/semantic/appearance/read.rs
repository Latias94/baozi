//! Direct appearance row reads sharing the validated transcript decoders.

use super::{
    CanonicalRecordView, NodeSceneAppearanceBinding, NodeSceneAppearanceCursor,
    NodeSceneAppearanceImage, NodeSceneAppearanceMaterial, NodeSceneAppearanceSampler,
    NodeSceneAppearanceTables, NodeSceneAppearanceTexture, NodeSceneSemanticResult,
    NodeSceneValidationProof, appearance_binding_field, appearance_image_field,
    appearance_material_field, invariant, read_u32_as_usize,
};

impl<'scene> NodeSceneAppearanceTables<'scene> {
    pub(in crate::canonical_scene::store) fn image_at(
        self,
        proof: &'scene NodeSceneValidationProof,
        index: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAppearanceImage<'scene>> {
        let table = require_row(self.images, index, proof.image_count())?;
        let mut cursor = self.into_row_cursor(proof);
        cursor.next_image = index;
        cursor.next_image_payload_offset = read_u32_as_usize(
            table,
            index,
            appearance_image_field::PAYLOAD_OFFSET,
            "validated image payload offset is unreadable",
        )?;
        cursor.next_media_type_offset = read_u32_as_usize(
            table,
            index,
            appearance_image_field::MEDIA_TYPE_OFFSET,
            "validated image media-type offset is unreadable",
        )?;
        cursor
            .next_image()?
            .ok_or_else(|| invariant("validated image row is absent"))
    }

    pub(in crate::canonical_scene::store) fn sampler_at(
        self,
        proof: &'scene NodeSceneValidationProof,
        index: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAppearanceSampler> {
        let mut cursor = self.into_row_cursor(proof);
        cursor.next_sampler = index;
        cursor
            .next_sampler()?
            .ok_or_else(|| invariant("validated sampler row is absent"))
    }

    pub(in crate::canonical_scene::store) fn texture_at(
        self,
        proof: &'scene NodeSceneValidationProof,
        index: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAppearanceTexture> {
        let mut cursor = self.into_row_cursor(proof);
        cursor.next_texture = index;
        cursor
            .next_texture()?
            .ok_or_else(|| invariant("validated texture row is absent"))
    }

    pub(in crate::canonical_scene::store) fn material_at(
        self,
        proof: &'scene NodeSceneValidationProof,
        index: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAppearanceMaterial<'scene>> {
        let table = require_row(self.materials, index, proof.material_count())?;
        let mut cursor = self.into_row_cursor(proof);
        cursor.next_material = index;
        cursor.next_material_binding_offset = read_u32_as_usize(
            table,
            index,
            appearance_material_field::BINDING_OFFSET,
            "validated material binding offset is unreadable",
        )?;
        cursor.next_material_factor_offset = read_u32_as_usize(
            table,
            index,
            appearance_material_field::FACTOR_OFFSET,
            "validated material factor offset is unreadable",
        )?;
        cursor.next_material_name_offset = read_u32_as_usize(
            table,
            index,
            appearance_material_field::NAME_OFFSET,
            "validated material name offset is unreadable",
        )?;
        cursor
            .next_material()?
            .ok_or_else(|| invariant("validated material row is absent"))
    }

    pub(in crate::canonical_scene::store) fn binding_at(
        self,
        proof: &'scene NodeSceneValidationProof,
        index: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAppearanceBinding<'scene>> {
        let table = require_row(self.bindings, index, proof.material_binding_count())?;
        let mut cursor = self.into_row_cursor(proof);
        cursor.next_binding = index;
        cursor.next_binding_semantic_offset = read_u32_as_usize(
            table,
            index,
            appearance_binding_field::SEMANTIC_SCALAR_OFFSET,
            "validated material-binding semantic offset is unreadable",
        )?;
        cursor.next_source_key_offset = read_u32_as_usize(
            table,
            index,
            appearance_binding_field::SOURCE_KEY_OFFSET,
            "validated material-binding source-key offset is unreadable",
        )?;
        cursor
            .next_binding()?
            .ok_or_else(|| invariant("validated material-binding row is absent"))
    }

    fn into_row_cursor(
        self,
        proof: &'scene NodeSceneValidationProof,
    ) -> NodeSceneAppearanceCursor<'scene> {
        // The Scene proof already establishes table order and contiguous ranges. Mark the
        // unrelated phases complete without traversing them; each caller selects exactly one
        // row and seeds its recorded offsets before invoking the shared single-row decoder.
        let mut cursor = self.into_cursor(proof);
        cursor.next_image = proof.image_count();
        cursor.next_sampler = proof.sampler_count();
        cursor.next_texture = proof.texture_count();
        cursor.next_material = proof.material_count();
        cursor.next_binding = proof.material_binding_count();
        cursor
    }
}

fn require_row(
    table: Option<CanonicalRecordView<'_>>,
    index: usize,
    count: usize,
) -> NodeSceneSemanticResult<CanonicalRecordView<'_>> {
    if index >= count {
        return Err(invariant(
            "appearance row index is outside its proof-qualified table",
        ));
    }
    table.ok_or_else(|| invariant("validated appearance table view is absent"))
}
