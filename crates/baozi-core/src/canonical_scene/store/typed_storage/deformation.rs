//! Deformation rows and their directly borrowed payloads in one retained Scene owner.
//!
//! All indices and ranges are logical. Sharing this owner between Scene generations never
//! requires rewriting a concrete-store slot or projecting an influence or morph payload.

use crate::canonical_scene::store::production::input::deformation::MorphTargetIdentity;
use crate::deform::{MorphDeltaSemantic, SkinMatrixSpaces};
use crate::math::Mat4;

use super::{StoredLogicalMeshIndex, StoredNodeIndex, TableRange, TextRange};

pub(in crate::canonical_scene::store) mod build;
pub(in crate::canonical_scene::store) mod validation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct StoredSkinIndex(
    pub(in crate::canonical_scene::store) u32,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct StoredMorphTargetIndex(
    pub(in crate::canonical_scene::store) u32,
);

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredSkin {
    pub(in crate::canonical_scene::store) name: Option<TextRange>,
    pub(in crate::canonical_scene::store) joints: TableRange<StoredNodeIndex>,
    pub(in crate::canonical_scene::store) inverse_bind_matrices: TableRange<Mat4>,
    pub(in crate::canonical_scene::store) skeleton_root: Option<StoredNodeIndex>,
    /// Includes the optional correction inline. Finite affine matrices retain authored bits,
    /// including shear, reflection, singular scale, and signed zero.
    pub(in crate::canonical_scene::store) matrix_spaces: SkinMatrixSpaces,
}

/// Row order is exactly the geometry logical-mesh order, including empty morph domains.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredLogicalMorph {
    pub(in crate::canonical_scene::store) definitions: TableRange<StoredMorphDefinition>,
    pub(in crate::canonical_scene::store) default_weights: TableRange<f32>,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredMorphDefinition {
    pub(in crate::canonical_scene::store) logical_mesh: StoredLogicalMeshIndex,
    pub(in crate::canonical_scene::store) target_ordinal: u32,
    pub(in crate::canonical_scene::store) stable_identity: MorphTargetIdentity,
    pub(in crate::canonical_scene::store) name: Option<TextRange>,
}

/// A primitive-local offset stream and one common range into all three influence columns.
/// Offsets remain relative to this primitive's influence slice, not the shared pools.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredInfluenceRows {
    pub(in crate::canonical_scene::store) vertex_offsets: TableRange<u32>,
    pub(in crate::canonical_scene::store) influences: TableRange<u32>,
}

/// Row order is exactly the geometry primitive order. Some(empty) is distinct from None.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredPrimitiveDeformation {
    pub(in crate::canonical_scene::store) influences: Option<StoredInfluenceRows>,
    pub(in crate::canonical_scene::store) morph_targets: TableRange<StoredMorphTarget>,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredMorphTarget {
    pub(in crate::canonical_scene::store) primitive: u32,
    pub(in crate::canonical_scene::store) target_ordinal: u32,
    pub(in crate::canonical_scene::store) deltas: TableRange<StoredMorphDelta>,
}

/// Sparse omissions mean a zero delta. Explicit zero tuples and empty sparse streams are
/// retained, and no default tuple is allocated or inferred from explicit values.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) enum StoredMorphDeltaPayload {
    Dense(TableRange<f32>),
    Sparse {
        logical_count: u32,
        indices: TableRange<u32>,
        values: TableRange<f32>,
    },
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredMorphDelta {
    pub(in crate::canonical_scene::store) target: StoredMorphTargetIndex,
    pub(in crate::canonical_scene::store) semantic: MorphDeltaSemantic,
    pub(in crate::canonical_scene::store) payload: StoredMorphDeltaPayload,
}

/// Row order is exactly the geometry mesh-instance order. An authored empty override is
/// Some(empty), independently of whether a skin is selected.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredMeshInstanceDeformation {
    pub(in crate::canonical_scene::store) skin: Option<StoredSkinIndex>,
    pub(in crate::canonical_scene::store) morph_weights: Option<TableRange<f32>>,
}

/// Seven row tables and ten compact payload buffers, admitted and retained together.
/// Storage drops before the enclosing retained-storage reservation; there is no nested owner.
pub(in crate::canonical_scene::store) struct DeformationStorage {
    pub(in crate::canonical_scene::store) skins: Vec<StoredSkin>,
    pub(in crate::canonical_scene::store) logical_morphs: Vec<StoredLogicalMorph>,
    pub(in crate::canonical_scene::store) morph_definitions: Vec<StoredMorphDefinition>,
    pub(in crate::canonical_scene::store) primitives: Vec<StoredPrimitiveDeformation>,
    pub(in crate::canonical_scene::store) morph_targets: Vec<StoredMorphTarget>,
    pub(in crate::canonical_scene::store) morph_deltas: Vec<StoredMorphDelta>,
    pub(in crate::canonical_scene::store) mesh_instances: Vec<StoredMeshInstanceDeformation>,
    pub(in crate::canonical_scene::store) joints: Vec<StoredNodeIndex>,
    pub(in crate::canonical_scene::store) inverse_bind_matrices: Vec<Mat4>,
    /// Skin names followed by morph-definition names, without terminators or interning.
    pub(in crate::canonical_scene::store) names: Vec<u8>,
    pub(in crate::canonical_scene::store) influence_vertex_offsets: Vec<u32>,
    pub(in crate::canonical_scene::store) influence_joints: Vec<u32>,
    pub(in crate::canonical_scene::store) influence_weights: Vec<f32>,
    pub(in crate::canonical_scene::store) influence_source_orders: Vec<u32>,
    /// Logical-mesh default weights followed by present mesh-instance overrides.
    pub(in crate::canonical_scene::store) morph_weights: Vec<f32>,
    pub(in crate::canonical_scene::store) morph_values: Vec<f32>,
    pub(in crate::canonical_scene::store) morph_sparse_indices: Vec<u32>,
}
