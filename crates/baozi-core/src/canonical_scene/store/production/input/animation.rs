use crate::animation::{AnimationExtrapolation, AnimationInterpolation, AnimationTimeDomain};

use super::geometry::AuthoringRange;
use super::{NodeLocalId, Presence};

/// Dense target identity within one source-shaped authoring batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnimationTargetAuthoring {
    NodeTranslation(NodeLocalId),
    NodeRotation(NodeLocalId),
    NodeScale(NodeLocalId),
    MeshInstanceMorphWeights(u32),
}

/// One typed channel payload. Target semantics determine `value_width`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationChannelAuthoring<'a> {
    source_index: u32,
    target: AnimationTargetAuthoring,
    interpolation: AnimationInterpolation,
    pre_extrapolation: AnimationExtrapolation,
    post_extrapolation: AnimationExtrapolation,
    times: &'a [f64],
    values: &'a [f32],
    in_tangents: Presence<&'a [f32]>,
    out_tangents: Presence<&'a [f32]>,
}

impl<'a> AnimationChannelAuthoring<'a> {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        source_index: u32,
        target: AnimationTargetAuthoring,
        interpolation: AnimationInterpolation,
        pre_extrapolation: AnimationExtrapolation,
        post_extrapolation: AnimationExtrapolation,
        times: &'a [f64],
        values: &'a [f32],
        in_tangents: Presence<&'a [f32]>,
        out_tangents: Presence<&'a [f32]>,
    ) -> Self {
        Self {
            source_index,
            target,
            interpolation,
            pre_extrapolation,
            post_extrapolation,
            times,
            values,
            in_tangents,
            out_tangents,
        }
    }

    pub(in crate::canonical_scene::store) const fn source_index(self) -> u32 {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn target(self) -> AnimationTargetAuthoring {
        self.target
    }

    pub(in crate::canonical_scene::store) const fn interpolation(self) -> AnimationInterpolation {
        self.interpolation
    }

    pub(in crate::canonical_scene::store) const fn pre_extrapolation(
        self,
    ) -> AnimationExtrapolation {
        self.pre_extrapolation
    }

    pub(in crate::canonical_scene::store) const fn post_extrapolation(
        self,
    ) -> AnimationExtrapolation {
        self.post_extrapolation
    }

    pub(in crate::canonical_scene::store) const fn times(self) -> &'a [f64] {
        self.times
    }

    pub(in crate::canonical_scene::store) const fn values(self) -> &'a [f32] {
        self.values
    }

    pub(in crate::canonical_scene::store) const fn in_tangents(self) -> Presence<&'a [f32]> {
        self.in_tangents
    }

    pub(in crate::canonical_scene::store) const fn out_tangents(self) -> Presence<&'a [f32]> {
        self.out_tangents
    }
}

/// One source animation clip retaining a contiguous range of source channels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationClipAuthoring<'a> {
    source_index: u32,
    name: Presence<&'a str>,
    duration: f64,
    time_domain: AnimationTimeDomain,
    channels: AuthoringRange,
}

impl<'a> AnimationClipAuthoring<'a> {
    pub const fn new(
        source_index: u32,
        duration: f64,
        time_domain: AnimationTimeDomain,
        channels: AuthoringRange,
    ) -> Self {
        Self {
            source_index,
            name: Presence::Absent,
            duration,
            time_domain,
            channels,
        }
    }

    /// Borrows an authored name without copying, including an explicitly authored empty string.
    ///
    /// Submission validates the display text and meters its retained UTF-8 bytes.
    pub const fn with_name(mut self, name: &'a str) -> Self {
        self.name = Presence::Present(name);
        self
    }

    pub(in crate::canonical_scene::store) const fn name(self) -> Presence<&'a str> {
        self.name
    }

    pub(in crate::canonical_scene::store) const fn source_index(self) -> u32 {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn duration(self) -> f64 {
        self.duration
    }

    pub(in crate::canonical_scene::store) const fn time_domain(self) -> AnimationTimeDomain {
        self.time_domain
    }

    pub(in crate::canonical_scene::store) const fn channels(self) -> AuthoringRange {
        self.channels
    }
}

/// Borrowed animation-domain input supplied by a format adapter.
#[derive(Debug, Clone, Copy)]
pub struct AnimationAuthoringBatch<'a> {
    clips: &'a [AnimationClipAuthoring<'a>],
    channels: &'a [AnimationChannelAuthoring<'a>],
}

impl<'a> AnimationAuthoringBatch<'a> {
    pub const fn empty() -> Self {
        Self::new(&[], &[])
    }

    pub const fn new(
        clips: &'a [AnimationClipAuthoring<'a>],
        channels: &'a [AnimationChannelAuthoring<'a>],
    ) -> Self {
        Self { clips, channels }
    }

    pub(in crate::canonical_scene::store) const fn clips(self) -> &'a [AnimationClipAuthoring<'a>] {
        self.clips
    }

    pub(in crate::canonical_scene::store) const fn channels(
        self,
    ) -> &'a [AnimationChannelAuthoring<'a>] {
        self.channels
    }
}
