//! Animation rows and directly borrowed scalar pools in one retained Scene owner.
//!
//! Rows contain logical, generation-independent indices and ranges only. The owner can
//! be shared by later Scene generations without rewriting a concrete-store reference.

use crate::animation::{AnimationExtrapolation, AnimationInterpolation, AnimationTimeDomain};

use super::{StoredNodeIndex, TableRange, TextRange};

pub(in crate::canonical_scene::store) mod build;
pub(in crate::canonical_scene::store) mod validation;

/// Zero-based logical row in GeometryStorage::mesh_instances. Unlike StoredNodeIndex,
/// this domain has no synthetic root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::canonical_scene::store) struct StoredMeshInstanceIndex(
    pub(in crate::canonical_scene::store) u32,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum StoredAnimationTarget {
    NodeTranslation(StoredNodeIndex),
    NodeRotation(StoredNodeIndex),
    NodeScale(StoredNodeIndex),
    MeshInstanceMorphWeights(StoredMeshInstanceIndex),
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredAnimationClip {
    pub(in crate::canonical_scene::store) source_index: u32,
    pub(in crate::canonical_scene::store) name: Option<TextRange>,
    pub(in crate::canonical_scene::store) duration: f64,
    pub(in crate::canonical_scene::store) time_domain: AnimationTimeDomain,
    pub(in crate::canonical_scene::store) channels: TableRange<StoredAnimationChannel>,
}

/// Scalars are key-major, with cubic derivatives in separate pools in the declared
/// time domain. No width, key-count, or scalar-kind tag duplicates the actual ranges.
/// TRS width follows the target; morph width follows values.len()/times.len(). Central
/// validation must additionally compare morph width with the targeted instance's
/// actual logical-mesh morph definitions and verify cubic tangent presence and shape.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredAnimationChannel {
    pub(in crate::canonical_scene::store) source_index: u32,
    pub(in crate::canonical_scene::store) target: StoredAnimationTarget,
    pub(in crate::canonical_scene::store) interpolation: AnimationInterpolation,
    pub(in crate::canonical_scene::store) pre_extrapolation: AnimationExtrapolation,
    pub(in crate::canonical_scene::store) post_extrapolation: AnimationExtrapolation,
    pub(in crate::canonical_scene::store) times: TableRange<f64>,
    pub(in crate::canonical_scene::store) values: TableRange<f32>,
    pub(in crate::canonical_scene::store) in_tangents: Option<TableRange<f32>>,
    pub(in crate::canonical_scene::store) out_tangents: Option<TableRange<f32>>,
}

impl StoredAnimationChannel {
    /// Shape-derived width only; this does not validate pool bounds, finite values,
    /// target ownership, or the instance's actual morph width.
    pub(in crate::canonical_scene::store) fn value_width(self) -> Option<u32> {
        let keys = self.times.len();
        if keys == 0 || !self.values.len().is_multiple_of(keys) {
            return None;
        }
        let width = u32::try_from(self.values.len() / keys).ok()?;
        match self.target {
            StoredAnimationTarget::NodeTranslation(_) | StoredAnimationTarget::NodeScale(_)
                if width == 3 =>
            {
                Some(width)
            }
            StoredAnimationTarget::NodeRotation(_) if width == 4 => Some(width),
            StoredAnimationTarget::MeshInstanceMorphWeights(_) if width != 0 => Some(width),
            _ => None,
        }
    }
}

/// Exactly two row tables and five compact pools. All allocations belong to this
/// value; operation's retained-storage owner accounts for and releases the whole value.
pub(in crate::canonical_scene::store) struct AnimationStorage {
    pub(in crate::canonical_scene::store) clips: Vec<StoredAnimationClip>,
    pub(in crate::canonical_scene::store) channels: Vec<StoredAnimationChannel>,
    pub(in crate::canonical_scene::store) times: Vec<f64>,
    pub(in crate::canonical_scene::store) values: Vec<f32>,
    pub(in crate::canonical_scene::store) in_tangents: Vec<f32>,
    pub(in crate::canonical_scene::store) out_tangents: Vec<f32>,
    pub(in crate::canonical_scene::store) names: Vec<u8>,
}
