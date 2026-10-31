//! Direct semantic views over the proof-qualified typed animation owner.
//!
//! Random reads select one typed row and borrow only its checked ranges. Central validation owns numeric
//! sample and cross-domain correspondence checks; exhaustive reads additionally check ordered
//! table coverage and exact use of every scalar/name pool. No record codec or row reset remains.

use crate::animation::{AnimationExtrapolation, AnimationInterpolation, AnimationTimeDomain};
use crate::canonical_scene::store::typed_storage::animation::{
    AnimationStorage, StoredAnimationTarget,
};
use crate::canonical_scene::store::typed_storage::{StoredNodeIndex, TableRange};
use crate::canonical_scene::store::validation::NodeSceneValidationProof;

use super::{NodeSceneSemanticResult, invariant};

mod cursor;
mod read;
#[cfg(test)]
mod tests;

pub(in crate::canonical_scene::store) use cursor::NodeSceneAnimationCursor;

#[cfg(test)]
std::thread_local! {
    static ROWS_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn take_row_read_count_for_test() -> usize {
    ROWS_READ.with(|count| count.replace(0))
}

fn record_row_read() {
    #[cfg(test)]
    ROWS_READ.with(|count| count.set(count.get() + 1));
}

/// A borrowed owner and two scalar target-domain bounds, without retaining or copying a proof.
/// The closed stored targets already resolve node/instance identity. Actual TRS eligibility and
/// morph width correspondence belong to the immutable Scene's central validation proof.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneAnimationTables<'scene> {
    storage: &'scene AnimationStorage,
    source_node_count: usize,
    mesh_instance_count: usize,
}

impl<'scene> NodeSceneAnimationTables<'scene> {
    pub(in crate::canonical_scene::store) fn try_new(
        storage: &'scene AnimationStorage,
        proof: &NodeSceneValidationProof,
    ) -> NodeSceneSemanticResult<Self> {
        if storage.clips.len() != proof.animation_clip_count()
            || storage.channels.len() != proof.animation_channel_count()
            || storage.in_tangents.len() != storage.out_tangents.len()
        {
            return Err(invariant(
                "typed animation tables disagree with central validation proof",
            ));
        }
        Ok(Self {
            storage,
            source_node_count: proof.source_node_count(),
            mesh_instance_count: proof.mesh_instance_count(),
        })
    }

    pub(super) fn into_cursor(self) -> NodeSceneAnimationCursor<'scene> {
        NodeSceneAnimationCursor::new(self)
    }
}

fn borrow_range<T>(range: TableRange<T>, pool: &[T]) -> NodeSceneSemanticResult<&[T]> {
    range
        .get(pool)
        .ok_or_else(|| invariant("validated animation payload range is unavailable"))
}

fn borrow_optional_range<T>(
    range: Option<TableRange<T>>,
    pool: &[T],
) -> NodeSceneSemanticResult<Option<&[T]>> {
    range.map(|range| borrow_range(range, pool)).transpose()
}

/// Header scalars retain their authored precision; the name borrows the actual UTF-8 range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneAnimationClip<'scene> {
    name: Option<&'scene str>,
    source_index: usize,
    duration: f64,
    time_domain: AnimationTimeDomain,
    channel_offset: usize,
    channel_count: usize,
}

impl<'scene> NodeSceneAnimationClip<'scene> {
    pub(in crate::canonical_scene::store) const fn name(self) -> Option<&'scene str> {
        self.name
    }

    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn duration(self) -> f64 {
        self.duration
    }

    pub(in crate::canonical_scene::store) const fn time_domain(self) -> AnimationTimeDomain {
        self.time_domain
    }

    pub(in crate::canonical_scene::store) const fn channel_offset(self) -> usize {
        self.channel_offset
    }

    pub(in crate::canonical_scene::store) const fn channel_count(self) -> usize {
        self.channel_count
    }
}

/// Node variants retain canonical rows (row zero is synthetic); instances are zero-based.
/// These are the existing snapshot target semantics, independent of the storage representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneAnimationTarget {
    NodeTranslation(usize),
    NodeRotation(usize),
    NodeScale(usize),
    MeshInstanceMorphWeights(usize),
}

/// Key-major samples borrow the Scene's actual scalar pools, including separate cubic tangents.
/// Snapshot and public readers consume these slices directly, without tuple projection or copies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneAnimationChannel<'scene> {
    source_index: usize,
    target: NodeSceneAnimationTarget,
    interpolation: AnimationInterpolation,
    pre_extrapolation: AnimationExtrapolation,
    post_extrapolation: AnimationExtrapolation,
    value_width: usize,
    times: &'scene [f64],
    values: &'scene [f32],
    in_tangents: Option<&'scene [f32]>,
    out_tangents: Option<&'scene [f32]>,
}

impl<'scene> NodeSceneAnimationChannel<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn target(self) -> NodeSceneAnimationTarget {
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

    pub(in crate::canonical_scene::store) const fn key_count(self) -> usize {
        self.times.len()
    }

    pub(in crate::canonical_scene::store) const fn value_width(self) -> usize {
        self.value_width
    }

    pub(in crate::canonical_scene::store) const fn times(self) -> &'scene [f64] {
        self.times
    }

    pub(in crate::canonical_scene::store) const fn values(self) -> &'scene [f32] {
        self.values
    }

    pub(in crate::canonical_scene::store) const fn in_tangents(self) -> Option<&'scene [f32]> {
        self.in_tangents
    }

    pub(in crate::canonical_scene::store) const fn out_tangents(self) -> Option<&'scene [f32]> {
        self.out_tangents
    }
}
