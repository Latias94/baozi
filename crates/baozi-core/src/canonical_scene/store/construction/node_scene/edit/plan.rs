use thiserror::Error;

use crate::canonical_scene::store::schema::{
    NODE_SCENE_MATRIX_SCALAR_COUNT, NODE_SCENE_TRS_SCALAR_COUNT,
};
use crate::entity::{AuthoredTransform, AuthoredTransformKind};

use super::super::{SceneEvidenceCounts, ValidatedNodeScene};
use super::CanonicalInvalidations;

/// A count proven to fit both retained-store plans and canonical `u32` fields.
#[derive(Debug, Clone, Copy)]
pub(super) struct CheckedCount {
    usize_value: usize,
    u32_value: u32,
}

impl CheckedCount {
    fn try_from_usize(
        value: usize,
        quantity: &'static str,
    ) -> Result<Self, NodeTransformEditPlanError> {
        let u32_value = u32::try_from(value)
            .map_err(|_| NodeTransformEditPlanError::SizeOverflow { quantity })?;
        Ok(Self {
            usize_value: value,
            u32_value,
        })
    }

    fn try_from_u32(
        value: u32,
        quantity: &'static str,
    ) -> Result<Self, NodeTransformEditPlanError> {
        let usize_value = usize::try_from(value)
            .map_err(|_| NodeTransformEditPlanError::SizeOverflow { quantity })?;
        Ok(Self {
            usize_value,
            u32_value: value,
        })
    }

    pub(super) const fn as_usize(self) -> usize {
        self.usize_value
    }

    pub(super) const fn as_u32(self) -> u32 {
        self.u32_value
    }
}

/// Logical target coordinates and the exact transform replacement interval.
///
/// `canonical_row` is a logical node-table row, not a retained-store slot identity.
#[derive(Debug, Clone, Copy)]
pub(super) struct NodeTransformEditTarget {
    pub(super) source_index: CheckedCount,
    pub(super) canonical_row: CheckedCount,
    pub(super) transform_offset: CheckedCount,
    pub(super) old_transform_scalars: CheckedCount,
    pub(super) new_transform_scalars: CheckedCount,
}

impl NodeTransformEditTarget {
    fn try_new(
        source_index: usize,
        canonical_row: usize,
        transform_offset: usize,
        old_transform: AuthoredTransform,
        replacement: AuthoredTransform,
    ) -> Result<Self, NodeTransformEditPlanError> {
        let expected_row =
            source_index
                .checked_add(1)
                .ok_or(NodeTransformEditPlanError::SizeOverflow {
                    quantity: "target canonical node row",
                })?;
        if canonical_row != expected_row {
            return Err(NodeTransformEditPlanError::InvalidSource {
                reason: "target source index does not map to the next canonical row",
            });
        }

        Ok(Self {
            source_index: CheckedCount::try_from_usize(source_index, "target source node index")?,
            canonical_row: CheckedCount::try_from_usize(
                canonical_row,
                "target canonical node row",
            )?,
            transform_offset: CheckedCount::try_from_usize(
                transform_offset,
                "target authored-transform offset",
            )?,
            old_transform_scalars: CheckedCount::try_from_usize(
                authored_transform_scalar_count(old_transform),
                "old authored-transform scalar count",
            )?,
            new_transform_scalars: CheckedCount::try_from_usize(
                authored_transform_scalar_count(replacement),
                "new authored-transform scalar count",
            )?,
        })
    }

    pub(super) const fn requires_node_shape_rewrite(self) -> bool {
        self.old_transform_scalars.as_usize() != self.new_transform_scalars.as_usize()
    }
}

/// Every immutable cardinality needed to plan later operation-owned materialization.
///
/// Counts are fixed before an operation starts, so admission never has to rediscover
/// semantic shape or perform a fallible integer conversion after mutation begins.
#[derive(Clone, Copy)]
pub(super) struct NodeTransformEditFixedCounts {
    pub(super) evidence: SceneEvidenceCounts,
    pub(super) canonical_blocks: CheckedCount,
    /// Blocks whose domain values are directly affected by the transform edit.
    ///
    /// This is a semantic rewrite count, not physical new-allocation authority.
    pub(super) custom_semantic_rewrite_blocks: CheckedCount,
    /// Every target block that must be newly retained for the destination generation.
    pub(super) new_canonical_blocks: CheckedCount,
    /// The only source blocks admitted for cross-generation sharing.
    pub(super) inherited_payload_leaves: CheckedCount,
    pub(super) canonical_attribution_slots: CheckedCount,
    pub(super) outcome_evidence_blocks: CheckedCount,
    pub(super) outcome_evidence_attribution_slots: CheckedCount,
    pub(super) canonical_node_rows: CheckedCount,
    pub(super) source_node_rows: CheckedCount,
    pub(super) source_root_rows: CheckedCount,
    pub(super) source_edge_rows: CheckedCount,
    pub(super) source_reference_rows: CheckedCount,
    pub(super) source_transform_scalars: CheckedCount,
    pub(super) output_transform_scalars: CheckedCount,
    pub(super) images: CheckedCount,
    pub(super) samplers: CheckedCount,
    pub(super) textures: CheckedCount,
    pub(super) materials: CheckedCount,
    pub(super) material_bindings: CheckedCount,
    pub(super) logical_meshes: CheckedCount,
    pub(super) primitives: CheckedCount,
    pub(super) attributes: CheckedCount,
    pub(super) mesh_instances: CheckedCount,
    pub(super) skins: CheckedCount,
    pub(super) logical_morphs: CheckedCount,
    pub(super) morph_definitions: CheckedCount,
    pub(super) primitive_deformations: CheckedCount,
    pub(super) morph_targets: CheckedCount,
    pub(super) morph_deltas: CheckedCount,
    pub(super) animation_clips: CheckedCount,
    pub(super) animation_channels: CheckedCount,
    pub(super) payload_blocks: CheckedCount,
}

impl NodeTransformEditFixedCounts {
    fn try_new(
        input: &ValidatedNodeScene,
        requires_node_shape_rewrite: bool,
        source_node_rows: usize,
        source_root_rows: usize,
        source_edge_rows: usize,
        source_transform_scalars: usize,
        output_transform_scalars: usize,
    ) -> Result<Self, NodeTransformEditPlanError> {
        let evidence = input.proof.counts();
        let canonical_blocks = CheckedCount::try_from_usize(
            input.canonical_block_count(),
            "canonical source block count",
        )?;
        let evidence_canonical_blocks = CheckedCount::try_from_u32(
            evidence.canonical_block_count,
            "evidence canonical block count",
        )?;
        require_count(
            canonical_blocks.as_usize(),
            evidence_canonical_blocks.as_usize(),
            "physical canonical block count disagrees with validation evidence",
        )?;

        let source_node_rows =
            CheckedCount::try_from_usize(source_node_rows, "source node row count")?;
        let source_root_rows =
            CheckedCount::try_from_usize(source_root_rows, "source root row count")?;
        let source_edge_rows =
            CheckedCount::try_from_usize(source_edge_rows, "source edge row count")?;
        let canonical_node_rows =
            CheckedCount::try_from_u32(evidence.node_count, "canonical node row count")?;
        let expected_canonical_node_rows = source_node_rows.as_usize().checked_add(1).ok_or(
            NodeTransformEditPlanError::SizeOverflow {
                quantity: "canonical node row count",
            },
        )?;
        require_count(
            canonical_node_rows.as_usize(),
            expected_canonical_node_rows,
            "canonical node count does not include exactly one synthetic root",
        )?;
        let partitioned_source_rows = source_root_rows
            .as_usize()
            .checked_add(source_edge_rows.as_usize())
            .ok_or(NodeTransformEditPlanError::SizeOverflow {
                quantity: "source root-edge partition",
            })?;
        require_count(
            partitioned_source_rows,
            source_node_rows.as_usize(),
            "source root and edge counts do not partition the source nodes",
        )?;

        let payload_blocks =
            CheckedCount::try_from_u32(evidence.payload_block_count, "payload block count")?;
        let custom_semantic_rewrite_block_count = if requires_node_shape_rewrite { 3 } else { 1 };
        let inherited_payload_leaf_count = payload_blocks.as_usize();
        let new_canonical_block_count = canonical_blocks
            .as_usize()
            .checked_sub(inherited_payload_leaf_count)
            .ok_or(NodeTransformEditPlanError::InvalidSource {
                reason: "payload block count exceeds the canonical source block count",
            })?;
        if custom_semantic_rewrite_block_count > new_canonical_block_count {
            return Err(NodeTransformEditPlanError::InvalidSource {
                reason: "canonical source lacks the structural blocks required by a transform edit",
            });
        }
        let custom_semantic_rewrite_blocks = CheckedCount::try_from_usize(
            custom_semantic_rewrite_block_count,
            "custom semantic rewrite block count",
        )?;
        let new_canonical_blocks =
            CheckedCount::try_from_usize(new_canonical_block_count, "new canonical block count")?;
        let canonical_attribution_slots = CheckedCount::try_from_usize(
            new_canonical_block_count.checked_add(1).ok_or(
                NodeTransformEditPlanError::SizeOverflow {
                    quantity: "canonical attribution slot count",
                },
            )?,
            "canonical attribution slot count",
        )?;

        Ok(Self {
            evidence,
            canonical_blocks,
            custom_semantic_rewrite_blocks,
            new_canonical_blocks,
            inherited_payload_leaves: CheckedCount::try_from_usize(
                inherited_payload_leaf_count,
                "inherited payload leaf count",
            )?,
            canonical_attribution_slots,
            outcome_evidence_blocks: CheckedCount::try_from_u32(1, "outcome-evidence block count")?,
            outcome_evidence_attribution_slots: CheckedCount::try_from_u32(
                2,
                "outcome-evidence attribution slot count",
            )?,
            canonical_node_rows,
            source_node_rows,
            source_root_rows,
            source_edge_rows,
            source_reference_rows: source_node_rows,
            source_transform_scalars: CheckedCount::try_from_usize(
                source_transform_scalars,
                "source authored-transform scalar count",
            )?,
            output_transform_scalars: CheckedCount::try_from_usize(
                output_transform_scalars,
                "output authored-transform scalar count",
            )?,
            images: CheckedCount::try_from_u32(evidence.image_count, "image row count")?,
            samplers: CheckedCount::try_from_u32(evidence.sampler_count, "sampler row count")?,
            textures: CheckedCount::try_from_u32(evidence.texture_count, "texture row count")?,
            materials: CheckedCount::try_from_u32(evidence.material_count, "material row count")?,
            material_bindings: CheckedCount::try_from_u32(
                evidence.material_binding_count,
                "material binding row count",
            )?,
            logical_meshes: CheckedCount::try_from_u32(
                evidence.logical_mesh_count,
                "logical mesh row count",
            )?,
            primitives: CheckedCount::try_from_u32(
                evidence.primitive_count,
                "primitive row count",
            )?,
            attributes: CheckedCount::try_from_u32(
                evidence.attribute_count,
                "attribute row count",
            )?,
            mesh_instances: CheckedCount::try_from_u32(
                evidence.mesh_instance_count,
                "mesh instance row count",
            )?,
            skins: CheckedCount::try_from_u32(evidence.skin_count, "skin row count")?,
            logical_morphs: CheckedCount::try_from_u32(
                evidence.logical_morph_count,
                "logical morph row count",
            )?,
            morph_definitions: CheckedCount::try_from_u32(
                evidence.morph_definition_count,
                "morph definition row count",
            )?,
            primitive_deformations: CheckedCount::try_from_u32(
                evidence.primitive_deformation_count,
                "primitive deformation row count",
            )?,
            morph_targets: CheckedCount::try_from_u32(
                evidence.morph_target_count,
                "morph target row count",
            )?,
            morph_deltas: CheckedCount::try_from_u32(
                evidence.morph_delta_count,
                "morph delta row count",
            )?,
            animation_clips: CheckedCount::try_from_u32(
                evidence.animation_clip_count,
                "animation clip row count",
            )?,
            animation_channels: CheckedCount::try_from_u32(
                evidence.animation_channel_count,
                "animation channel row count",
            )?,
            payload_blocks,
        })
    }
}

/// A changed transform edit whose source and immutable shape remain borrowed.
#[must_use = "a changed node transform edit must be materialized or deliberately dropped"]
pub(in crate::canonical_scene::store) struct PendingNodeTransformEdit<'scene> {
    pub(super) input: &'scene ValidatedNodeScene,
    pub(super) target: NodeTransformEditTarget,
    pub(super) replacement: AuthoredTransform,
    pub(super) counts: NodeTransformEditFixedCounts,
    pub(super) invalidations: CanonicalInvalidations,
}

impl<'scene> PendingNodeTransformEdit<'scene> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn try_new(
        input: &'scene ValidatedNodeScene,
        target_source_index: usize,
        target_canonical_row: usize,
        target_transform_offset: usize,
        old_transform: AuthoredTransform,
        replacement: AuthoredTransform,
        source_node_rows: usize,
        source_root_rows: usize,
        source_edge_rows: usize,
        source_transform_scalars: usize,
    ) -> Result<Self, NodeTransformEditPlanError> {
        let target = NodeTransformEditTarget::try_new(
            target_source_index,
            target_canonical_row,
            target_transform_offset,
            old_transform,
            replacement,
        )?;
        if target.source_index.as_usize() >= source_node_rows {
            return Err(NodeTransformEditPlanError::InvalidSource {
                reason: "target source node index exceeds the proof-qualified source rows",
            });
        }
        let target_end = target
            .transform_offset
            .as_usize()
            .checked_add(target.old_transform_scalars.as_usize())
            .ok_or(NodeTransformEditPlanError::SizeOverflow {
                quantity: "target authored-transform range",
            })?;
        if target_end > source_transform_scalars {
            return Err(NodeTransformEditPlanError::InvalidSource {
                reason: "target authored-transform range exceeds the source sequence",
            });
        }
        let output_transform_scalars = source_transform_scalars
            .checked_sub(target.old_transform_scalars.as_usize())
            .and_then(|count| count.checked_add(target.new_transform_scalars.as_usize()))
            .ok_or(NodeTransformEditPlanError::SizeOverflow {
                quantity: "output authored-transform scalar count",
            })?;
        let counts = NodeTransformEditFixedCounts::try_new(
            input,
            target.requires_node_shape_rewrite(),
            source_node_rows,
            source_root_rows,
            source_edge_rows,
            source_transform_scalars,
            output_transform_scalars,
        )?;

        Ok(Self {
            input,
            target,
            replacement,
            counts,
            invalidations: CanonicalInvalidations::scene_bounds_invalidated(),
        })
    }

    pub(in crate::canonical_scene::store) const fn input(&self) -> &'scene ValidatedNodeScene {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn replacement(&self) -> AuthoredTransform {
        self.replacement
    }

    pub(in crate::canonical_scene::store) const fn invalidations(&self) -> CanonicalInvalidations {
        self.invalidations
    }
}

#[derive(Debug, Error)]
pub(super) enum NodeTransformEditPlanError {
    #[error("validated node Scene cannot support a transform edit: {reason}")]
    InvalidSource { reason: &'static str },
    #[error("node transform edit size overflowed while computing {quantity}")]
    SizeOverflow { quantity: &'static str },
}

pub(super) const fn authored_transform_scalar_count(transform: AuthoredTransform) -> usize {
    match transform.kind() {
        AuthoredTransformKind::Matrix => NODE_SCENE_MATRIX_SCALAR_COUNT,
        AuthoredTransformKind::Trs => NODE_SCENE_TRS_SCALAR_COUNT,
    }
}

fn require_count(
    actual: usize,
    expected: usize,
    reason: &'static str,
) -> Result<(), NodeTransformEditPlanError> {
    if actual == expected {
        Ok(())
    } else {
        Err(NodeTransformEditPlanError::InvalidSource { reason })
    }
}

#[cfg(test)]
mod tests {
    use crate::entity::AuthoredTransform;
    use crate::math::Mat4;

    use super::{NodeTransformEditPlanError, NodeTransformEditTarget};

    #[test]
    fn matrix_to_trs_uses_checked_six_scalar_contraction() {
        let old = AuthoredTransform::try_matrix(Mat4::IDENTITY)
            .expect("identity matrix must be a valid authored transform");
        let target =
            NodeTransformEditTarget::try_new(0, 1, 0, old, AuthoredTransform::identity_trs())
                .expect("the first source node must map to canonical row one");

        assert_eq!(target.old_transform_scalars.as_usize(), 16);
        assert_eq!(target.old_transform_scalars.as_u32(), 16);
        assert_eq!(target.new_transform_scalars.as_usize(), 10);
        assert_eq!(target.new_transform_scalars.as_u32(), 10);
        assert!(target.requires_node_shape_rewrite());
    }

    #[test]
    fn same_transform_layout_does_not_rewrite_node_shape() {
        let target = NodeTransformEditTarget::try_new(
            0,
            1,
            0,
            AuthoredTransform::identity_trs(),
            AuthoredTransform::try_trs(
                crate::math::Vec3::new(1.0, 2.0, 3.0),
                crate::math::Quat::IDENTITY,
                crate::math::Vec3::new(1.0, 1.0, 1.0),
            )
            .expect("finite TRS components must be accepted"),
        )
        .expect("the first source node must map to canonical row one");

        assert!(!target.requires_node_shape_rewrite());
    }

    #[test]
    fn target_coordinates_reject_a_non_synthetic_row_offset() {
        let error = NodeTransformEditTarget::try_new(
            0,
            0,
            0,
            AuthoredTransform::identity_trs(),
            AuthoredTransform::identity_trs(),
        )
        .expect_err("source index zero cannot address canonical row zero");

        assert!(matches!(
            error,
            NodeTransformEditPlanError::InvalidSource { .. }
        ));
    }
}
