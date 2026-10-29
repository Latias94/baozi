use super::*;

// Seed the exhaustive decoder from this row's exact offsets. The immutable Scene has
// already validated owner coverage and payload contiguity; random reads need no prefix scan.
impl<'scene> NodeSceneAnimationTables<'scene> {
    pub(in crate::canonical_scene::store) fn clip_at(
        self,
        proof: NodeSceneValidationProof,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAnimationClip<'scene>> {
        let table = self
            .clips
            .ok_or_else(|| invariant("animation-clip table is absent"))?;
        let offset = read_u32_as_usize(
            table,
            row,
            animation_clip_field::CHANNEL_OFFSET,
            "animation-clip channel offset is unreadable",
        )?;
        let name_offset = read_u32_as_usize(
            table,
            row,
            animation_clip_field::NAME_OFFSET,
            "animation-clip name offset is unreadable",
        )?;
        let mut cursor = self.into_cursor(proof);
        cursor.next_clip = row;
        cursor.next_clip_channel_offset = offset;
        cursor.next_clip_name_offset = name_offset;
        cursor
            .next_clip()?
            .ok_or_else(|| invariant("animation-clip row is absent"))
    }

    pub(in crate::canonical_scene::store) fn channel_at(
        self,
        proof: NodeSceneValidationProof,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAnimationChannel<'scene>> {
        let table = self
            .channels
            .ok_or_else(|| invariant("animation-channel table is absent"))?;
        let offset =
            |field| read_u32_as_usize(table, row, field, "animation-channel offset is unreadable");
        let time_offset = offset(animation_channel_field::TIME_OFFSET)?;
        let value_offset = offset(animation_channel_field::VALUE_OFFSET)?;
        let in_tangent_offset = offset(animation_channel_field::IN_TANGENT_OFFSET)?;
        let out_tangent_offset = offset(animation_channel_field::OUT_TANGENT_OFFSET)?;
        let mut cursor = self.into_cursor(proof);
        cursor.phase = AnimationCursorPhase::Channels;
        cursor.next_clip = proof.animation_clip_count();
        cursor.next_clip_channel_offset = proof.animation_channel_count();
        cursor.next_channel = row;
        cursor.next_time_offset = time_offset;
        cursor.next_value_offset = value_offset;
        cursor.next_in_tangent_offset = in_tangent_offset;
        cursor.next_out_tangent_offset = out_tangent_offset;
        cursor
            .next_channel()?
            .ok_or_else(|| invariant("animation-channel row is absent"))
    }
}
