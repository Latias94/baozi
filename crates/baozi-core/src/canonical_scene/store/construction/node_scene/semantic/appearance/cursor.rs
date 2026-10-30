//! Exact table, owner-range and shared-text coverage in the existing snapshot phase order.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AppearanceCursorPhase {
    Images,
    Samplers,
    Textures,
    Materials,
    Bindings,
}

#[must_use = "the appearance cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceCursor<'scene> {
    tables: NodeSceneAppearanceTables<'scene>,
    phase: AppearanceCursorPhase,
    next_image: usize,
    next_sampler: usize,
    next_texture: usize,
    next_material: usize,
    next_binding: usize,
    image_bytes: usize,
    media_bytes: usize,
    owned_bindings: usize,
    name_start: Option<usize>,
    name_end: usize,
    source_key_bytes: usize,
    last_source_key_offset: usize,
}

impl<'scene> NodeSceneAppearanceCursor<'scene> {
    pub(super) fn new(tables: NodeSceneAppearanceTables<'scene>) -> Self {
        Self {
            tables,
            phase: AppearanceCursorPhase::Images,
            next_image: 0,
            next_sampler: 0,
            next_texture: 0,
            next_material: 0,
            next_binding: 0,
            image_bytes: 0,
            media_bytes: 0,
            owned_bindings: 0,
            name_start: None,
            name_end: 0,
            source_key_bytes: 0,
            last_source_key_offset: 0,
        }
    }

    pub(in crate::canonical_scene::store) fn image_count(&self) -> usize {
        self.tables.storage.images.len()
    }

    pub(in crate::canonical_scene::store) fn sampler_count(&self) -> usize {
        self.tables.storage.samplers.len()
    }

    pub(in crate::canonical_scene::store) fn texture_count(&self) -> usize {
        self.tables.storage.textures.len()
    }

    pub(in crate::canonical_scene::store) fn material_count(&self) -> usize {
        self.tables.storage.materials.len()
    }

    pub(in crate::canonical_scene::store) fn binding_count(&self) -> usize {
        self.tables.storage.bindings.len()
    }

    pub(in crate::canonical_scene::store) fn next_image(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAppearanceImage<'scene>>> {
        self.enter_phase(AppearanceCursorPhase::Images)?;
        if self.next_image == self.image_count() {
            return Ok(None);
        }
        let row = self.next_image;
        let view = self.tables.image_at(row)?;
        let stored = &self.tables.storage.images[row];
        consume(stored.payload, &mut self.image_bytes)?;
        if let Some(media) = stored.media_type {
            consume(media.0, &mut self.media_bytes)?;
        }
        self.next_image += 1;
        Ok(Some(view))
    }

    pub(in crate::canonical_scene::store) fn next_sampler(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAppearanceSampler>> {
        self.enter_phase(AppearanceCursorPhase::Samplers)?;
        if self.next_sampler == self.sampler_count() {
            return Ok(None);
        }
        let view = self.tables.sampler_at(self.next_sampler)?;
        self.next_sampler += 1;
        Ok(Some(view))
    }

    pub(in crate::canonical_scene::store) fn next_texture(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAppearanceTexture>> {
        self.enter_phase(AppearanceCursorPhase::Textures)?;
        if self.next_texture == self.texture_count() {
            return Ok(None);
        }
        let view = self.tables.texture_at(self.next_texture)?;
        self.next_texture += 1;
        Ok(Some(view))
    }

    pub(in crate::canonical_scene::store) fn next_material(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAppearanceMaterial<'scene>>> {
        self.enter_phase(AppearanceCursorPhase::Materials)?;
        if self.next_material == self.material_count() {
            return Ok(None);
        }
        let row = self.next_material;
        let view = self.tables.material_at(row)?;
        let stored = &self.tables.storage.materials[row];
        consume(stored.bindings, &mut self.owned_bindings)?;
        if let Some(name) = stored.name {
            // Names are last in the shared text pool, but precede bindings in snapshot traversal.
            // Remember their first boundary and close it against the source-key end at finish.
            if self.name_start.is_none() {
                self.name_start = Some(name.0.start());
                self.name_end = name.0.start();
            }
            consume(name.0, &mut self.name_end)?;
        }
        self.next_material += 1;
        Ok(Some(view))
    }

    pub(in crate::canonical_scene::store) fn next_binding(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAppearanceBinding<'scene>>> {
        self.enter_phase(AppearanceCursorPhase::Bindings)?;
        if self.owned_bindings != self.binding_count() {
            return Err(invariant(
                "validated material ranges do not partition all bindings",
            ));
        }
        if self.next_binding == self.binding_count() {
            return Ok(None);
        }
        let row = self.next_binding;
        let view = self.tables.binding_at(row)?;
        let offset = self.source_key_bytes;
        if let Some(key) = self.tables.storage.bindings[row].source_key {
            let mut absolute = self
                .media_bytes
                .checked_add(offset)
                .ok_or_else(|| invariant("appearance source-key text boundary overflows"))?;
            consume(key.0, &mut absolute)?;
            self.source_key_bytes = absolute
                .checked_sub(self.media_bytes)
                .ok_or_else(|| invariant("appearance source-key text boundary is invalid"))?;
        }
        self.last_source_key_offset = offset;
        self.next_binding += 1;
        Ok(Some(view))
    }

    /// Offset of the most recently yielded binding in the snapshot's virtual source-key pool.
    /// This belongs to ordered traversal: absent keys retain the preceding byte offset without
    /// adding redundant offsets or section lengths to the actual stored rows.
    pub(in crate::canonical_scene::store) const fn source_key_offset(&self) -> usize {
        self.last_source_key_offset
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        let storage = self.tables.storage;
        let key_end = self
            .media_bytes
            .checked_add(self.source_key_bytes)
            .ok_or_else(|| invariant("appearance text boundary overflows"))?;
        let text_complete = match self.name_start {
            Some(start) => start == key_end && self.name_end == storage.text.len(),
            None => key_end == storage.text.len(),
        };
        if self.next_image != self.image_count()
            || self.next_sampler != self.sampler_count()
            || self.next_texture != self.texture_count()
            || self.next_material != self.material_count()
            || self.next_binding != self.binding_count()
            || self.image_bytes != storage.image_bytes.len()
            || self.owned_bindings != self.binding_count()
            || !text_complete
        {
            return Err(invariant(
                "appearance cursor ended before all tables and pools were exhausted",
            ));
        }
        Ok(())
    }

    fn enter_phase(&mut self, target: AppearanceCursorPhase) -> NodeSceneSemanticResult<()> {
        if target < self.phase {
            return Err(invariant(
                "appearance cursor cannot return to an earlier table phase",
            ));
        }
        if (target >= AppearanceCursorPhase::Samplers && self.next_image != self.image_count())
            || (target >= AppearanceCursorPhase::Textures
                && self.next_sampler != self.sampler_count())
            || (target >= AppearanceCursorPhase::Materials
                && self.next_texture != self.texture_count())
            || (target >= AppearanceCursorPhase::Bindings
                && self.next_material != self.material_count())
        {
            return Err(invariant(
                "appearance collection phases must be exhausted in order",
            ));
        }
        self.phase = target;
        Ok(())
    }
}

fn consume<T>(range: TableRange<T>, cursor: &mut usize) -> NodeSceneSemanticResult<()> {
    if range.start() != *cursor {
        return Err(invariant("validated appearance ranges are not contiguous"));
    }
    *cursor = cursor
        .checked_add(range.len())
        .ok_or_else(|| invariant("appearance range cursor overflows"))?;
    Ok(())
}
