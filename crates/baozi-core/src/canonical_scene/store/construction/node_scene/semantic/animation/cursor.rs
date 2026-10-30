//! Ordered clip/channel traversal and exact shared-pool coverage, with no retained proof.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AnimationCursorPhase {
    Clips,
    Channels,
}

#[must_use = "the animation cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneAnimationCursor<'scene> {
    tables: NodeSceneAnimationTables<'scene>,
    phase: AnimationCursorPhase,
    next_clip: usize,
    next_channel: usize,
    owned_channels: usize,
    names: usize,
    times: usize,
    values: usize,
    in_tangents: usize,
    out_tangents: usize,
}

impl<'scene> NodeSceneAnimationCursor<'scene> {
    pub(super) fn new(tables: NodeSceneAnimationTables<'scene>) -> Self {
        Self {
            tables,
            phase: AnimationCursorPhase::Clips,
            next_clip: 0,
            next_channel: 0,
            owned_channels: 0,
            names: 0,
            times: 0,
            values: 0,
            in_tangents: 0,
            out_tangents: 0,
        }
    }

    pub(in crate::canonical_scene::store) fn clip_count(&self) -> usize {
        self.tables.storage.clips.len()
    }

    pub(in crate::canonical_scene::store) fn channel_count(&self) -> usize {
        self.tables.storage.channels.len()
    }

    pub(in crate::canonical_scene::store) fn next_clip(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAnimationClip<'scene>>> {
        if self.phase != AnimationCursorPhase::Clips {
            return Err(invariant(
                "animation cursor cannot return to the clip phase",
            ));
        }
        if self.next_clip == self.clip_count() {
            return Ok(None);
        }
        let row = self.next_clip;
        let view = self.tables.clip_at(row)?;
        let clip = &self.tables.storage.clips[row];
        if row > 0 && self.tables.storage.clips[row - 1].source_index >= clip.source_index {
            return Err(invariant(
                "validated animation clip source indices are not strictly ordered",
            ));
        }
        consume(clip.channels, &mut self.owned_channels)?;
        if let Some(name) = clip.name {
            consume(name.0, &mut self.names)?;
        }
        self.next_clip += 1;
        Ok(Some(view))
    }

    pub(in crate::canonical_scene::store) fn next_channel(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAnimationChannel<'scene>>> {
        if self.next_clip != self.clip_count() || self.owned_channels != self.channel_count() {
            return Err(invariant(
                "animation clips must exhaust and partition the channel table before channels",
            ));
        }
        self.phase = AnimationCursorPhase::Channels;
        if self.next_channel == self.channel_count() {
            return Ok(None);
        }
        let row = self.next_channel;
        let view = self.tables.channel_at(row)?;
        let channel = &self.tables.storage.channels[row];
        if row > 0 && self.tables.storage.channels[row - 1].source_index >= channel.source_index {
            return Err(invariant(
                "validated animation channel source indices are not strictly ordered",
            ));
        }
        consume(channel.times, &mut self.times)?;
        consume(channel.values, &mut self.values)?;
        if let Some(input) = channel.in_tangents {
            consume(input, &mut self.in_tangents)?;
        }
        if let Some(output) = channel.out_tangents {
            consume(output, &mut self.out_tangents)?;
        }
        self.next_channel += 1;
        Ok(Some(view))
    }

    /// Every collection phase must have yielded all rows. Because scalar APIs return borrowed
    /// slices, exact exhaustion here means every pool range was covered once by those rows.
    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        let storage = self.tables.storage;
        if self.next_clip != storage.clips.len()
            || self.next_channel != storage.channels.len()
            || self.owned_channels != storage.channels.len()
            || self.names != storage.names.len()
            || self.times != storage.times.len()
            || self.values != storage.values.len()
            || self.in_tangents != storage.in_tangents.len()
            || self.out_tangents != storage.out_tangents.len()
        {
            return Err(invariant(
                "animation cursor ended before all tables and payload ranges were exhausted",
            ));
        }
        Ok(())
    }
}

fn consume<T>(range: TableRange<T>, cursor: &mut usize) -> NodeSceneSemanticResult<()> {
    if range.start() != *cursor {
        return Err(invariant("validated animation ranges are not contiguous"));
    }
    *cursor = cursor
        .checked_add(range.len())
        .ok_or_else(|| invariant("animation range cursor overflows"))?;
    Ok(())
}
