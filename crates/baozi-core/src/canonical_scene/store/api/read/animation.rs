use std::fmt;

use super::super::super::construction::node_scene::semantic::animation::{
    NodeSceneAnimationChannel, NodeSceneAnimationClip, NodeSceneAnimationTables,
    NodeSceneAnimationTarget,
};
use super::{ValidatedScene, checked_index, expect_validated};
use crate::animation::{AnimationExtrapolation, AnimationInterpolation, AnimationTimeDomain};
use crate::identity::{
    AnimationClipId, MeshInstanceId, NodeId, SceneEntityKind, SceneGenerationId, SceneLookupError,
};

fn tables(scene: &ValidatedScene) -> NodeSceneAnimationTables<'_> {
    expect_validated(NodeSceneAnimationTables::try_new(
        scene.as_node_scene().read_animation(),
        scene.as_node_scene().read_proof(),
    ))
}

/// One channel's property and generation-qualified target in the same Scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationTarget {
    NodeTranslation(NodeId),
    NodeRotation(NodeId),
    NodeScale(NodeId),
    MeshInstanceMorphWeights(MeshInstanceId),
}

/// One animation clip borrowing its authored data from the immutable Scene.
#[derive(Clone, Copy)]
pub struct AnimationClipRef<'s> {
    scene: &'s ValidatedScene,
    index: usize,
}

impl<'s> AnimationClipRef<'s> {
    pub(super) const fn at(scene: &'s ValidatedScene, index: usize) -> Self {
        Self { scene, index }
    }

    fn read(self) -> NodeSceneAnimationClip<'s> {
        expect_validated(tables(self.scene).clip_at(self.index))
    }

    pub fn id(self) -> AnimationClipId {
        AnimationClipId::new(self.scene.generation(), self.index as u32)
    }

    /// Preserves an absent name and an explicitly authored empty name.
    pub fn name(self) -> Option<&'s str> {
        self.read().name()
    }

    /// Duration in the authored time domain, preserving its f64 precision.
    pub fn duration(self) -> f64 {
        self.read().duration()
    }

    pub fn time_domain(self) -> AnimationTimeDomain {
        self.read().time_domain()
    }

    pub fn channel_count(self) -> usize {
        self.read().channel_count()
    }

    /// Iterates channels in retained source order with independent iterator state.
    pub fn channels(
        self,
    ) -> impl ExactSizeIterator<Item = AnimationChannelRef<'s>> + DoubleEndedIterator + Clone + 's
    {
        let clip = self.read();
        let start = clip.channel_offset();
        (start..start + clip.channel_count())
            .map(move |row| AnimationChannelRef::at(self.scene, row))
    }

    /// Reads a channel by its index within this clip, without visiting preceding rows.
    pub fn channel(self, index: usize) -> Option<AnimationChannelRef<'s>> {
        let clip = self.read();
        (index < clip.channel_count())
            .then(|| AnimationChannelRef::at(self.scene, clip.channel_offset() + index))
    }
}

impl fmt::Debug for AnimationClipRef<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AnimationClipRef")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

/// One channel with borrowed, key-major payloads. Each key has `value_width()` f32 scalars;
/// cubic in/out tangents have the same shape and remain separate from the key values.
#[derive(Debug, Clone, Copy)]
pub struct AnimationChannelRef<'s> {
    generation: SceneGenerationId,
    channel: NodeSceneAnimationChannel<'s>,
}

impl<'s> AnimationChannelRef<'s> {
    fn at(scene: &'s ValidatedScene, row: usize) -> Self {
        Self {
            generation: scene.generation(),
            channel: expect_validated(tables(scene).channel_at(row)),
        }
    }

    pub fn target(self) -> AnimationTarget {
        let node = |row: usize| NodeId::new(self.generation, (row - 1) as u32);
        match self.channel.target() {
            NodeSceneAnimationTarget::NodeTranslation(row) => {
                AnimationTarget::NodeTranslation(node(row))
            }
            NodeSceneAnimationTarget::NodeRotation(row) => AnimationTarget::NodeRotation(node(row)),
            NodeSceneAnimationTarget::NodeScale(row) => AnimationTarget::NodeScale(node(row)),
            NodeSceneAnimationTarget::MeshInstanceMorphWeights(row) => {
                AnimationTarget::MeshInstanceMorphWeights(MeshInstanceId::new(
                    self.generation,
                    row as u32,
                ))
            }
        }
    }

    pub fn interpolation(self) -> AnimationInterpolation {
        self.channel.interpolation()
    }

    pub fn pre_extrapolation(self) -> AnimationExtrapolation {
        self.channel.pre_extrapolation()
    }

    pub fn post_extrapolation(self) -> AnimationExtrapolation {
        self.channel.post_extrapolation()
    }

    pub const fn key_count(self) -> usize {
        self.channel.key_count()
    }

    /// Scalars per key: translation/scale use 3, rotation uses 4 (xyzw), and morph weights
    /// use the target mesh's logical morph-target count.
    pub const fn value_width(self) -> usize {
        self.channel.value_width()
    }

    pub const fn key_times(self) -> &'s [f64] {
        self.channel.times()
    }

    pub const fn values(self) -> &'s [f32] {
        self.channel.values()
    }

    pub const fn in_tangents(self) -> Option<&'s [f32]> {
        self.channel.in_tangents()
    }

    pub const fn out_tangents(self) -> Option<&'s [f32]> {
        self.channel.out_tangents()
    }
}

impl ValidatedScene {
    pub fn animation_clip_count(&self) -> usize {
        self.as_node_scene().read_proof().animation_clip_count()
    }

    pub fn animation_clips(
        &self,
    ) -> impl ExactSizeIterator<Item = AnimationClipRef<'_>> + DoubleEndedIterator + Clone {
        (0..self.animation_clip_count()).map(|index| AnimationClipRef::at(self, index))
    }

    /// Looks up a clip after checking its Scene generation and collection range.
    pub fn animation_clip(
        &self,
        id: AnimationClipId,
    ) -> Result<AnimationClipRef<'_>, SceneLookupError> {
        let index = checked_index(
            self,
            SceneEntityKind::AnimationClip,
            id.generation(),
            id.index(),
            self.animation_clip_count(),
        )?;
        Ok(AnimationClipRef::at(self, index))
    }
}
