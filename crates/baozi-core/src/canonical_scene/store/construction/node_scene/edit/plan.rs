use baozi_operation::{BytesPlan, RecordPlan, SequencePlan};
use thiserror::Error;

use crate::canonical_scene::store::schema::profile::{
    PROFILE_OPERATION_SCALAR_COUNT, PROFILE_OWNER_FIELD_COUNT, PROFILE_OWNER_NODE,
    PROFILE_PROVENANCE_SCALAR_COUNT, PROFILE_SEGMENT_FIELD_COUNT, ProfileLineageAttachmentSchema,
    profile_owner_field,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, NODE_SCENE_MATRIX_SCALAR_COUNT, NODE_SCENE_TRS_SCALAR_COUNT,
};
use crate::entity::{AuthoredTransform, AuthoredTransformKind};
use crate::provenance::{DIGEST_LEN, OperationIdentity};

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
    pub(super) inherited_shareable_leaves: CheckedCount,
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
    pub(super) cameras: CheckedCount,
    pub(super) lights: CheckedCount,
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
    pub(super) profile: NodeTransformEditProfilePlan,
}

/// Exact source coordinates and allocation plans for appending one transform lineage segment.
#[derive(Clone, Copy)]
pub(super) struct NodeTransformEditProfilePlan {
    pub(super) source_segment_count: CheckedCount,
    pub(super) output_segment_count: CheckedCount,
    pub(super) target_owner_row: CheckedCount,
    pub(super) target_attachment_offset: CheckedCount,
    pub(super) target_attachment_count: CheckedCount,
    pub(super) source_attachment_count: CheckedCount,
    pub(super) output_attachment_count: CheckedCount,
    pub(super) source_attachments: CanonicalBlockRef<ProfileLineageAttachmentSchema>,
    pub(super) fresh_provenance_index: CheckedCount,
    pub(super) attachments: SequencePlan<u32>,
    pub(super) parents: SequencePlan<u32>,
    pub(super) provenance: SequencePlan<u32>,
    pub(super) operation: SequencePlan<u32>,
    pub(super) digest: BytesPlan,
    pub(super) segments: RecordPlan,
    pub(super) owners: RecordPlan,
}

impl NodeTransformEditFixedCounts {
    fn try_new(
        input: &ValidatedNodeScene,
        target: NodeTransformEditTarget,
        source_node_rows: usize,
        source_root_rows: usize,
        source_edge_rows: usize,
        source_transform_scalars: usize,
        output_transform_scalars: usize,
    ) -> Result<Self, NodeTransformEditPlanError> {
        let source_evidence = input.proof.counts();
        let profile = NodeTransformEditProfilePlan::try_new(input, target, source_evidence)?;
        let mut evidence = source_evidence;
        evidence.canonical_block_count = checked_add_u32(
            evidence.canonical_block_count,
            4,
            "canonical output block count",
        )?;
        evidence.payload_block_count = checked_add_u32(
            evidence.payload_block_count,
            1,
            "canonical payload block count",
        )?;
        evidence.profile_segment_count =
            checked_add_u32(evidence.profile_segment_count, 1, "profile segment count")?;
        evidence.profile_provenance_count = checked_add_u32(
            evidence.profile_provenance_count,
            1,
            "profile provenance count",
        )?;
        evidence.profile_operation_count = checked_add_u32(
            evidence.profile_operation_count,
            1,
            "profile operation count",
        )?;
        evidence.profile_parent_count = checked_add_u32(
            evidence.profile_parent_count,
            profile.target_attachment_count.as_u32(),
            "profile parent count",
        )?;
        evidence.profile_attachment_count = profile.output_attachment_count.as_u32();
        evidence.shareable_leaf_block_count = checked_add_u32(
            evidence.shareable_leaf_block_count,
            4,
            "shareable canonical leaf block count",
        )?;

        let source_canonical_blocks = CheckedCount::try_from_usize(
            input.canonical_block_count(),
            "canonical source block count",
        )?;
        let evidence_canonical_blocks = CheckedCount::try_from_u32(
            source_evidence.canonical_block_count,
            "source evidence canonical block count",
        )?;
        require_count(
            source_canonical_blocks.as_usize(),
            evidence_canonical_blocks.as_usize(),
            "physical canonical block count disagrees with validation evidence",
        )?;
        let canonical_blocks = CheckedCount::try_from_u32(
            evidence.canonical_block_count,
            "canonical output block count",
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

        let custom_semantic_rewrite_block_count = if target.requires_node_shape_rewrite() {
            3
        } else {
            1
        };
        let inherited_shareable_leaf_count = usize::try_from(
            source_evidence
                .shareable_leaf_block_count
                .checked_sub(1)
                .ok_or(NodeTransformEditPlanError::InvalidSource {
                    reason: "validated profile has no attachment leaf to rewrite",
                })?,
        )
        .map_err(|_| NodeTransformEditPlanError::SizeOverflow {
            quantity: "shareable canonical leaf block count",
        })?;
        let new_canonical_block_count = canonical_blocks
            .as_usize()
            .checked_sub(inherited_shareable_leaf_count)
            .ok_or(NodeTransformEditPlanError::InvalidSource {
                reason: "inherited shareable leaf count exceeds the canonical output block count",
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
            inherited_shareable_leaves: CheckedCount::try_from_usize(
                inherited_shareable_leaf_count,
                "inherited shareable leaf count",
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
            cameras: CheckedCount::try_from_u32(evidence.camera_count, "camera row count")?,
            lights: CheckedCount::try_from_u32(evidence.light_count, "light row count")?,
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
            profile,
        })
    }
}

impl NodeTransformEditProfilePlan {
    fn try_new(
        input: &ValidatedNodeScene,
        target: NodeTransformEditTarget,
        evidence: SceneEvidenceCounts,
    ) -> Result<Self, NodeTransformEditPlanError> {
        let source_segment_count = CheckedCount::try_from_usize(
            input.proof.profile_segment_count(),
            "source profile segment count",
        )?;
        let output_segment_count = CheckedCount::try_from_usize(
            source_segment_count.as_usize().checked_add(1).ok_or(
                NodeTransformEditPlanError::SizeOverflow {
                    quantity: "output profile segment count",
                },
            )?,
            "output profile segment count",
        )?;
        let owner_count = usize::try_from(evidence.profile_owner_count).map_err(|_| {
            NodeTransformEditPlanError::SizeOverflow {
                quantity: "profile owner count",
            }
        })?;
        let target_owner_row = CheckedCount::try_from_usize(
            target.canonical_row.as_usize().checked_add(1).ok_or(
                NodeTransformEditPlanError::SizeOverflow {
                    quantity: "target profile owner row",
                },
            )?,
            "target profile owner row",
        )?;
        let owners = input
            .canonical
            .records(input.proof.profile_owners().into_store())
            .ok_or(NodeTransformEditPlanError::InvalidSource {
                reason: "validated profile owner table is unavailable or mistyped",
            })?;
        if owners.rows() != owner_count
            || owners.fields() != PROFILE_OWNER_FIELD_COUNT
            || target_owner_row.as_usize() >= owner_count
            || owners.scalar::<u8>(target_owner_row.as_usize(), profile_owner_field::OWNER_KIND)
                != Some(PROFILE_OWNER_NODE)
            || owners.scalar::<u32>(
                target_owner_row.as_usize(),
                profile_owner_field::OWNER_INDEX,
            ) != Some(target.canonical_row.as_u32())
        {
            return Err(NodeTransformEditPlanError::InvalidSource {
                reason: "target node does not map to its proof-qualified profile owner row",
            });
        }
        let target_attachment_offset = CheckedCount::try_from_u32(
            owners
                .scalar::<u32>(
                    target_owner_row.as_usize(),
                    profile_owner_field::ATTACHMENT_OFFSET,
                )
                .ok_or(NodeTransformEditPlanError::InvalidSource {
                    reason: "target profile attachment offset is unreadable",
                })?,
            "target profile attachment offset",
        )?;
        let target_attachment_count = CheckedCount::try_from_u32(
            owners
                .scalar::<u32>(
                    target_owner_row.as_usize(),
                    profile_owner_field::ATTACHMENT_COUNT,
                )
                .ok_or(NodeTransformEditPlanError::InvalidSource {
                    reason: "target profile attachment count is unreadable",
                })?,
            "target profile attachment count",
        )?;
        if target_attachment_count.as_usize() == 0 {
            return Err(NodeTransformEditPlanError::InvalidSource {
                reason: "target profile owner has no provenance attachment",
            });
        }
        let source_attachments = owners
            .sequence_ref::<u32, ProfileLineageAttachmentSchema>(
                target_owner_row.as_usize(),
                profile_owner_field::ATTACHMENTS,
            )
            .map(CanonicalBlockRef::from_store)
            .ok_or(NodeTransformEditPlanError::InvalidSource {
                reason: "target profile attachment reference is unreadable",
            })?;
        let attachments = input
            .canonical
            .sequence(source_attachments.into_store())
            .ok_or(NodeTransformEditPlanError::InvalidSource {
                reason: "validated profile attachment leaf is unavailable or mistyped",
            })?;
        let source_attachment_count = CheckedCount::try_from_u32(
            evidence.profile_attachment_count,
            "source profile attachment count",
        )?;
        let target_attachment_end = target_attachment_offset
            .as_usize()
            .checked_add(target_attachment_count.as_usize())
            .ok_or(NodeTransformEditPlanError::SizeOverflow {
                quantity: "target profile attachment range",
            })?;
        if attachments.as_slice().len() != source_attachment_count.as_usize()
            || target_attachment_end > source_attachment_count.as_usize()
        {
            return Err(NodeTransformEditPlanError::InvalidSource {
                reason: "target profile attachment range disagrees with validation evidence",
            });
        }
        let output_attachment_count = CheckedCount::try_from_usize(
            source_attachment_count
                .as_usize()
                .checked_sub(target_attachment_count.as_usize())
                .and_then(|count| count.checked_add(1))
                .ok_or(NodeTransformEditPlanError::SizeOverflow {
                    quantity: "output profile attachment count",
                })?,
            "output profile attachment count",
        )?;
        let fresh_provenance_index = CheckedCount::try_from_u32(
            evidence.profile_provenance_count,
            "fresh profile provenance index",
        )?;

        Ok(Self {
            source_segment_count,
            output_segment_count,
            target_owner_row,
            target_attachment_offset,
            target_attachment_count,
            source_attachment_count,
            output_attachment_count,
            source_attachments,
            fresh_provenance_index,
            attachments: SequencePlan::new(output_attachment_count.as_usize()).map_err(|_| {
                NodeTransformEditPlanError::SizeOverflow {
                    quantity: "output profile attachment allocation",
                }
            })?,
            parents: SequencePlan::new(target_attachment_count.as_usize()).map_err(|_| {
                NodeTransformEditPlanError::SizeOverflow {
                    quantity: "transform provenance parent allocation",
                }
            })?,
            provenance: SequencePlan::new(PROFILE_PROVENANCE_SCALAR_COUNT).map_err(|_| {
                NodeTransformEditPlanError::SizeOverflow {
                    quantity: "transform provenance row allocation",
                }
            })?,
            operation: SequencePlan::new(PROFILE_OPERATION_SCALAR_COUNT).map_err(|_| {
                NodeTransformEditPlanError::SizeOverflow {
                    quantity: "transform operation row allocation",
                }
            })?,
            digest: BytesPlan::new(DIGEST_LEN).map_err(|_| {
                NodeTransformEditPlanError::SizeOverflow {
                    quantity: "transform options digest allocation",
                }
            })?,
            segments: RecordPlan::new(output_segment_count.as_usize(), PROFILE_SEGMENT_FIELD_COUNT)
                .map_err(|_| NodeTransformEditPlanError::SizeOverflow {
                    quantity: "output profile segment table",
                })?,
            owners: RecordPlan::new(owner_count, PROFILE_OWNER_FIELD_COUNT).map_err(|_| {
                NodeTransformEditPlanError::SizeOverflow {
                    quantity: "output profile owner table",
                }
            })?,
        })
    }
}

/// A changed transform edit whose source and immutable shape remain borrowed.
#[must_use = "a changed node transform edit must be materialized or deliberately dropped"]
pub(in crate::canonical_scene::store) struct PendingNodeTransformEdit<'scene> {
    pub(super) input: &'scene ValidatedNodeScene,
    pub(super) target: NodeTransformEditTarget,
    pub(super) replacement: AuthoredTransform,
    pub(super) operation: OperationIdentity,
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
        operation: OperationIdentity,
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
            target,
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
            operation,
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

fn checked_add_u32(
    left: u32,
    right: u32,
    quantity: &'static str,
) -> Result<u32, NodeTransformEditPlanError> {
    left.checked_add(right)
        .ok_or(NodeTransformEditPlanError::SizeOverflow { quantity })
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
