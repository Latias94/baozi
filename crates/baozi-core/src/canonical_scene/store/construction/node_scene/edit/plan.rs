use baozi_operation::{BytesPlan, RecordPlan, SequencePlan, StorePlanError};
use thiserror::Error;

use crate::canonical_scene::store::profile_provenance::PROFILE_PROVENANCE_ADDRESS_SCALARS;
use crate::canonical_scene::store::schema::profile::{
    PROFILE_OPERATION_SCALAR_COUNT, PROFILE_OWNER_ATTRIBUTE, PROFILE_OWNER_FIELD_COUNT,
    PROFILE_OWNER_NODE, PROFILE_OWNER_PRIMITIVE, PROFILE_PROVENANCE_SCALAR_COUNT,
    PROFILE_SEGMENT_FIELD_COUNT, ProfileLineageAttachmentSchema, profile_owner_field,
};
use crate::canonical_scene::store::schema::{CanonicalBlockRef, SceneDirectorySchema};
use crate::canonical_scene::store::validation::ProfileMeteredCounts;
use crate::entity::AuthoredTransform;
use crate::identity::SceneGenerationId;
use crate::provenance::{DIGEST_LEN, OperationIdentity, OperationKind};

use super::super::scratch::NodeSceneCentralValidationPlan;
use super::super::{SceneEvidenceCounts, SceneStorage};
use super::leaf_plan::NodeSceneInheritedLeafScanPlan;
use super::report::CanonicalInvalidations;

const NODE_TRANSFORM_PROFILE_CANONICAL_BLOCK_DELTA: u32 = 4;
const NODE_TRANSFORM_PROFILE_PAYLOAD_LEAF_DELTA: u32 = 1;
const NODE_TRANSFORM_PROFILE_SHAREABLE_LEAF_DELTA: u32 = 4;

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
    ) -> Result<Self, NodeSceneEditPlanError> {
        let u32_value =
            u32::try_from(value).map_err(|_| NodeSceneEditPlanError::SizeOverflow { quantity })?;
        Ok(Self {
            usize_value: value,
            u32_value,
        })
    }

    fn try_from_u32(value: u32, quantity: &'static str) -> Result<Self, NodeSceneEditPlanError> {
        let usize_value = usize::try_from(value)
            .map_err(|_| NodeSceneEditPlanError::SizeOverflow { quantity })?;
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

/// Exact disposition of every evidence-counted shareable leaf across one node-Scene edit.
///
/// `SceneEvidenceCounts::shareable_leaf_block_count` covers payload and profile leaves. Record
/// blocks are always rebuilt, while operation-specific leaves such as authored transforms are
/// tracked by their operation plan. Keeping this boundary explicit makes deletion a core concern
/// instead of relying on a storage-wide "inherit everything else" operation.
#[derive(Debug, Clone, Copy)]
pub(super) struct NodeSceneEvidenceLeafPlan {
    pub(super) source: CheckedCount,
    pub(super) inherited: CheckedCount,
    pub(super) rewritten: CheckedCount,
    pub(super) added: CheckedCount,
    pub(super) deleted: CheckedCount,
    pub(super) output: CheckedCount,
}

impl NodeSceneEvidenceLeafPlan {
    fn try_new(
        source: u32,
        output: u32,
        rewritten: usize,
        added: usize,
        deleted: usize,
    ) -> Result<Self, NodeSceneEditPlanError> {
        let source = CheckedCount::try_from_u32(source, "source shareable leaf count")?;
        let output = CheckedCount::try_from_u32(output, "output shareable leaf count")?;
        let rewritten = CheckedCount::try_from_usize(rewritten, "rewritten shareable leaf count")?;
        let added = CheckedCount::try_from_usize(added, "added shareable leaf count")?;
        let deleted = CheckedCount::try_from_usize(deleted, "deleted shareable leaf count")?;
        let inherited_count = source
            .as_usize()
            .checked_sub(rewritten.as_usize())
            .and_then(|count| count.checked_sub(deleted.as_usize()))
            .ok_or(NodeSceneEditPlanError::InvalidSource {
                reason: "shareable leaf rewrites and deletions exceed the source leaf set",
            })?;
        let expected_output = inherited_count
            .checked_add(rewritten.as_usize())
            .and_then(|count| count.checked_add(added.as_usize()))
            .ok_or(NodeSceneEditPlanError::SizeOverflow {
                quantity: "output shareable leaf count",
            })?;
        require_count(
            output.as_usize(),
            expected_output,
            "output shareable leaf count disagrees with the sealed leaf dispositions",
        )?;

        Ok(Self {
            source,
            inherited: CheckedCount::try_from_usize(
                inherited_count,
                "inherited shareable leaf count",
            )?,
            rewritten,
            added,
            deleted,
            output,
        })
    }

    pub(super) fn is_exact(self) -> bool {
        self.inherited
            .as_usize()
            .checked_add(self.rewritten.as_usize())
            .and_then(|count| count.checked_add(self.deleted.as_usize()))
            == Some(self.source.as_usize())
            && self
                .inherited
                .as_usize()
                .checked_add(self.rewritten.as_usize())
                .and_then(|count| count.checked_add(self.added.as_usize()))
                == Some(self.output.as_usize())
    }
}

/// Logical target coordinates in the actual typed node table.
///
/// `canonical_row` is a logical node-table row, not a retained-store slot identity.
#[derive(Debug, Clone, Copy)]
pub(super) struct NodeTransformEditTarget {
    pub(super) source_index: CheckedCount,
    pub(super) canonical_row: CheckedCount,
}

impl NodeTransformEditTarget {
    fn try_new(source_index: usize, canonical_row: usize) -> Result<Self, NodeSceneEditPlanError> {
        let expected_row =
            source_index
                .checked_add(1)
                .ok_or(NodeSceneEditPlanError::SizeOverflow {
                    quantity: "target canonical node row",
                })?;
        if canonical_row != expected_row {
            return Err(NodeSceneEditPlanError::InvalidSource {
                reason: "target source index does not map to the next canonical row",
            });
        }

        Ok(Self {
            source_index: CheckedCount::try_from_usize(source_index, "target source node index")?,
            canonical_row: CheckedCount::try_from_usize(
                canonical_row,
                "target canonical node row",
            )?,
        })
    }
}

/// The exact authored mutation consumed by one retained node-Scene edit transaction.
///
/// Keeping the program concrete and closed makes every new edit kind update planning,
/// materialization, validation, remapping, and provenance together. It deliberately does not use
/// a backend trait or an arbitrary payload.
#[allow(
    clippy::large_enum_variant,
    reason = "the source-bound geometry plan lives in an admitted execution cell; unbound transform edits retain only their small rewrite"
)]
pub(super) enum NodeSceneEditProgram<'scene> {
    NodeTransform(NodeTransformRewrite),
    TriangulatePrimitive(
        crate::canonical_scene::store::typed_storage::edit::GeometryTriangulationPlan<'scene>,
    ),
}

#[derive(Debug, Clone, Copy)]
pub(super) struct NodeTransformRewrite {
    target: NodeTransformEditTarget,
    replacement: AuthoredTransform,
}

impl NodeTransformRewrite {
    pub(super) const fn new(
        target: NodeTransformEditTarget,
        replacement: AuthoredTransform,
    ) -> Self {
        Self {
            target,
            replacement,
        }
    }

    pub(super) const fn target(self) -> NodeTransformEditTarget {
        self.target
    }

    pub(super) const fn replacement(self) -> AuthoredTransform {
        self.replacement
    }
}

impl NodeSceneEditProgram<'_> {
    pub(super) const fn node_transform(
        target: NodeTransformEditTarget,
        replacement: AuthoredTransform,
    ) -> Self {
        Self::NodeTransform(NodeTransformRewrite::new(target, replacement))
    }

    pub(super) const fn profile_operation_code(&self) -> u8 {
        match self {
            Self::NodeTransform(_) => {
                crate::canonical_scene::store::schema::profile::PROFILE_OPERATION_TRANSFORM
            }
            Self::TriangulatePrimitive(_) => {
                crate::canonical_scene::store::schema::profile::PROFILE_OPERATION_SPLIT
            }
        }
    }

    pub(super) const fn operation_kind(&self) -> OperationKind {
        match self {
            Self::NodeTransform(_) => OperationKind::Transform,
            Self::TriangulatePrimitive(_) => OperationKind::Split,
        }
    }
}

/// Every immutable cardinality needed to plan later operation-owned materialization.
///
/// Counts are fixed before an operation starts, so admission never has to rediscover
/// semantic shape or perform a fallible integer conversion after mutation begins.
#[derive(Clone, Copy)]
pub(super) struct NodeSceneEditFixedCounts {
    pub(super) evidence: SceneEvidenceCounts,
    pub(super) central_validation_plan: NodeSceneCentralValidationPlan,
    pub(super) canonical_blocks: CheckedCount,
    /// Every target block that must be newly retained for the destination generation.
    pub(super) new_canonical_blocks: CheckedCount,
    pub(super) evidence_leaves: NodeSceneEvidenceLeafPlan,
    pub(super) inherited_leaf_scan: NodeSceneInheritedLeafScanPlan,
    pub(super) canonical_attribution_slots: CheckedCount,
    pub(super) outcome_evidence_blocks: CheckedCount,
    pub(super) outcome_evidence_attribution_slots: CheckedCount,
    pub(super) canonical_node_rows: CheckedCount,
    pub(super) source_node_rows: CheckedCount,
    pub(super) source_root_rows: CheckedCount,
    pub(super) source_edge_rows: CheckedCount,
    pub(super) source_reference_rows: CheckedCount,
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
    pub(super) profile: NodeSceneEditProfilePlan,
}

/// Exact source coordinates and allocation plans for appending one transform lineage segment.
#[derive(Clone, Copy)]
pub(super) struct NodeSceneEditProfilePlan {
    pub(super) source_segment_count: CheckedCount,
    pub(super) output_segment_count: CheckedCount,
    pub(super) target_owner_row: CheckedCount,
    pub(super) target_count: CheckedCount,
    targets: NodeSceneEditProfileTargets,
    pub(super) target_attachment_count: CheckedCount,
    pub(super) source_attachment_count: CheckedCount,
    pub(super) output_attachment_count: CheckedCount,
    pub(super) source_attachments: CanonicalBlockRef<ProfileLineageAttachmentSchema>,
    pub(super) fresh_provenance_index: CheckedCount,
    pub(super) attachments: SequencePlan<u64>,
    pub(super) parents: SequencePlan<u64>,
    pub(super) provenance: SequencePlan<u32>,
    pub(super) operation: SequencePlan<u32>,
    pub(super) digest: BytesPlan,
    pub(super) segments: RecordPlan,
    pub(super) owners: RecordPlan,
}

/// Closed owner selection; attributes retain their actual, separate source parent sets.
#[derive(Debug, Clone, Copy)]
enum NodeSceneEditProfileTargets {
    Node(u32),
    Triangulation { primitive: usize },
}

impl NodeSceneEditProfileTargets {
    fn selects(self, input: &SceneStorage, kind: u8, index: u32) -> bool {
        match self {
            Self::Node(node) => kind == PROFILE_OWNER_NODE && index == node,
            Self::Triangulation { primitive } => {
                if kind == PROFILE_OWNER_PRIMITIVE {
                    return index as usize == primitive;
                }
                if kind != PROFILE_OWNER_ATTRIBUTE {
                    return false;
                }
                let geometry = input.read_geometry();
                let Some(primitive) = geometry.primitives.get(primitive) else {
                    return false;
                };
                primitive
                    .attributes
                    .checked(geometry.attributes.len())
                    .is_some_and(|range| {
                        range.contains(&(index as usize))
                            && geometry
                                .attributes
                                .get(index as usize)
                                .is_some_and(|attribute| {
                                    matches!(
                                        attribute.domain,
                                        crate::attribute::AttributeDomain::Corner
                                            | crate::attribute::AttributeDomain::Face
                                    )
                                })
                    })
            }
        }
    }
}

impl NodeSceneEditFixedCounts {
    fn try_new_node_transform(
        input: &SceneStorage,
        program: &NodeSceneEditProgram<'_>,
        source_node_rows: usize,
        source_root_rows: usize,
        source_edge_rows: usize,
    ) -> Result<Self, NodeSceneEditPlanError> {
        Self::try_new_for_program(
            input,
            program,
            source_node_rows,
            source_root_rows,
            source_edge_rows,
            &mut || Ok(()),
        )
    }

    fn try_new_for_program(
        input: &SceneStorage,
        program: &NodeSceneEditProgram<'_>,
        source_node_rows: usize,
        source_root_rows: usize,
        source_edge_rows: usize,
        poll: &mut impl FnMut() -> Result<(), NodeSceneEditPlanError>,
    ) -> Result<Self, NodeSceneEditPlanError> {
        let targets = match program {
            NodeSceneEditProgram::NodeTransform(rewrite) => {
                NodeSceneEditProfileTargets::Node(rewrite.target().canonical_row.as_u32())
            }
            NodeSceneEditProgram::TriangulatePrimitive(geometry) => {
                NodeSceneEditProfileTargets::Triangulation {
                    primitive: geometry.target_row(),
                }
            }
        };
        let source_evidence = input.proof.counts();
        let profile = NodeSceneEditProfilePlan::try_new(input, targets, source_evidence, poll)?;
        let mut evidence = source_evidence;
        evidence.canonical_block_count = checked_add_u32(
            evidence.canonical_block_count,
            NODE_TRANSFORM_PROFILE_CANONICAL_BLOCK_DELTA,
            "canonical output block count",
        )?;
        evidence.payload_block_count = checked_add_u32(
            evidence.payload_block_count,
            NODE_TRANSFORM_PROFILE_PAYLOAD_LEAF_DELTA,
            "canonical payload block count",
        )?;
        evidence.profile_segment_count =
            checked_add_u32(evidence.profile_segment_count, 1, "profile segment count")?;
        evidence.profile_provenance_count = checked_add_u32(
            evidence.profile_provenance_count,
            profile.target_count.as_u32(),
            "profile provenance count",
        )?;
        evidence.profile_operation_count = checked_add_u32(
            evidence.profile_operation_count,
            profile.target_count.as_u32(),
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
            NODE_TRANSFORM_PROFILE_SHAREABLE_LEAF_DELTA,
            "shareable canonical leaf block count",
        )?;
        let source_central_validation = input.proof.central_validation_plan();
        let source_validation_scratch = source_central_validation.scratch_plan();
        let profile_counts = try_node_transform_profile_counts(
            source_validation_scratch.profile_counts(),
            source_evidence,
            evidence,
            profile,
        )?;
        let mut central_validation_plan =
            source_central_validation.try_rebind_node_transform(evidence, profile_counts)?;
        if let NodeSceneEditProgram::TriangulatePrimitive(geometry) = program {
            central_validation_plan = central_validation_plan.try_rebind_geometry(
                input
                    .read_geometry()
                    .validation_work()
                    .ok_or(StorePlanError::SizeOverflow)?,
                geometry.validation_work(),
            )?;
        }

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
            NodeSceneEditPlanError::SizeOverflow {
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
            .ok_or(NodeSceneEditPlanError::SizeOverflow {
                quantity: "source root-edge partition",
            })?;
        require_count(
            partitioned_source_rows,
            source_node_rows.as_usize(),
            "source root and edge counts do not partition the source nodes",
        )?;

        let evidence_leaves = NodeSceneEvidenceLeafPlan::try_new(
            source_evidence.shareable_leaf_block_count,
            evidence.shareable_leaf_block_count,
            1,
            4,
            0,
        )?;
        let inherited_leaf_scan = NodeSceneInheritedLeafScanPlan::try_new(
            input,
            profile.source_segment_count.as_usize(),
            evidence_leaves,
        )?;
        let new_canonical_block_count = canonical_blocks
            .as_usize()
            .checked_sub(evidence_leaves.inherited.as_usize())
            .ok_or(NodeSceneEditPlanError::InvalidSource {
                reason: "inherited shareable leaf count exceeds the canonical output block count",
            })?;
        let new_canonical_blocks =
            CheckedCount::try_from_usize(new_canonical_block_count, "new canonical block count")?;
        let canonical_attribution_slots = CheckedCount::try_from_usize(
            new_canonical_block_count.checked_add(1).ok_or(
                NodeSceneEditPlanError::SizeOverflow {
                    quantity: "canonical attribution slot count",
                },
            )?,
            "canonical attribution slot count",
        )?;

        Ok(Self {
            evidence,
            central_validation_plan,
            canonical_blocks,
            new_canonical_blocks,
            evidence_leaves,
            inherited_leaf_scan,
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

impl NodeSceneEditProfilePlan {
    fn try_new(
        input: &SceneStorage,
        targets: NodeSceneEditProfileTargets,
        evidence: SceneEvidenceCounts,
        poll: &mut impl FnMut() -> Result<(), NodeSceneEditPlanError>,
    ) -> Result<Self, NodeSceneEditPlanError> {
        let invalid = || NodeSceneEditPlanError::InvalidSource {
            reason: "profile owner attachments disagree with the sealed source",
        };
        let count = |value, quantity| CheckedCount::try_from_usize(value, quantity);
        let source_segment_count = count(input.proof.profile_segment_count(), "source segments")?;
        let output_segment_count = count(
            source_segment_count
                .as_usize()
                .checked_add(1)
                .ok_or_else(invalid)?,
            "output segments",
        )?;
        let owners = input
            .canonical
            .records(input.proof.profile_owners().into_store())
            .ok_or_else(invalid)?;
        if owners.rows() != evidence.profile_owner_count as usize
            || owners.fields() != PROFILE_OWNER_FIELD_COUNT
        {
            return Err(invalid());
        }
        let mut target_count = 0_usize;
        let mut target_parents = 0_usize;
        let mut first_target = None;
        let mut source_attachments = None;
        let mut next = 0_usize;
        for row in 0..owners.rows() {
            poll()?;
            let kind = owners
                .scalar::<u8>(row, profile_owner_field::OWNER_KIND)
                .ok_or_else(invalid)?;
            let index = owners
                .scalar::<u32>(row, profile_owner_field::OWNER_INDEX)
                .ok_or_else(invalid)?;
            let offset = owners
                .scalar::<u32>(row, profile_owner_field::ATTACHMENT_OFFSET)
                .ok_or_else(invalid)? as usize;
            let length = owners
                .scalar::<u32>(row, profile_owner_field::ATTACHMENT_COUNT)
                .ok_or_else(invalid)? as usize;
            let reference = owners
                .sequence_ref::<u64, ProfileLineageAttachmentSchema>(
                    row,
                    profile_owner_field::ATTACHMENTS,
                )
                .map(CanonicalBlockRef::from_store)
                .ok_or_else(invalid)?;
            if offset != next
                || length == 0
                || source_attachments.is_some_and(|previous| previous != reference)
            {
                return Err(invalid());
            }
            source_attachments = Some(reference);
            next = next.checked_add(length).ok_or_else(invalid)?;
            if targets.selects(input, kind, index) {
                first_target.get_or_insert(row);
                target_count = target_count.checked_add(1).ok_or_else(invalid)?;
                target_parents = target_parents.checked_add(length).ok_or_else(invalid)?;
            }
        }
        let source_attachments = source_attachments.ok_or_else(invalid)?;
        if next != evidence.profile_attachment_count as usize || target_count == 0 {
            return Err(invalid());
        }
        // The source proof fixes one owner per primitive and attribute; every selected owner is
        // traversed independently, never replaced by a union of another owner's parents.
        let attachments = input
            .canonical
            .sequence(source_attachments.into_store())
            .ok_or_else(invalid)?;
        if attachments.as_slice().len()
            != next
                .checked_mul(PROFILE_PROVENANCE_ADDRESS_SCALARS)
                .ok_or_else(invalid)?
        {
            return Err(invalid());
        }
        let output_count = next
            .checked_sub(target_parents)
            .and_then(|n| n.checked_add(target_count))
            .ok_or_else(invalid)?;
        Ok(Self {
            source_segment_count,
            output_segment_count,
            target_owner_row: count(
                first_target.ok_or_else(invalid)?,
                "first changed profile owner",
            )?,
            target_count: count(target_count, "changed profile owners")?,
            targets,
            target_attachment_count: count(target_parents, "changed owners' parent count")?,
            source_attachment_count: count(next, "source attachments")?,
            output_attachment_count: count(output_count, "output attachments")?,
            source_attachments,
            fresh_provenance_index: CheckedCount::try_from_u32(
                evidence.profile_provenance_count,
                "fresh provenance base",
            )?,
            attachments: SequencePlan::new(
                output_count
                    .checked_mul(PROFILE_PROVENANCE_ADDRESS_SCALARS)
                    .ok_or_else(invalid)?,
            )?,
            parents: SequencePlan::new(
                target_parents
                    .checked_mul(PROFILE_PROVENANCE_ADDRESS_SCALARS)
                    .ok_or_else(invalid)?,
            )?,
            provenance: SequencePlan::new(
                target_count
                    .checked_mul(PROFILE_PROVENANCE_SCALAR_COUNT)
                    .ok_or_else(invalid)?,
            )?,
            operation: SequencePlan::new(
                target_count
                    .checked_mul(PROFILE_OPERATION_SCALAR_COUNT)
                    .ok_or_else(invalid)?,
            )?,
            digest: BytesPlan::new(target_count.checked_mul(DIGEST_LEN).ok_or_else(invalid)?)?,
            segments: RecordPlan::new(
                output_segment_count.as_usize(),
                PROFILE_SEGMENT_FIELD_COUNT,
            )?,
            owners: RecordPlan::new(owners.rows(), PROFILE_OWNER_FIELD_COUNT)?,
        })
    }

    pub(super) fn selects(self, input: &SceneStorage, kind: u8, index: u32) -> bool {
        self.targets.selects(input, kind, index)
    }
}

/// A changed edit sealed against one exact input publication.
///
/// The plan deliberately does not borrow its source Scene. A post-process run can therefore own
/// the input Scene and this plan side by side without self-reference. Execution must bind the plan
/// back to that exact retained root before an operation, allocation, or provenance nonce is
/// created. The generation remains diagnostic identity; the destination-qualified root is the
/// storage identity.
#[must_use = "a changed node Scene edit must be materialized or deliberately dropped"]
pub(in crate::canonical_scene::store) struct PendingNodeSceneEdit {
    input_generation: SceneGenerationId,
    input_root: CanonicalBlockRef<SceneDirectorySchema>,
    rewrite: NodeTransformRewrite,
    pub(super) operation: OperationIdentity,
    pub(super) counts: NodeSceneEditFixedCounts,
    pub(super) invalidations: CanonicalInvalidations,
}

/// Execution-local pairing of a sealed edit plan with its exact retained source.
pub(super) struct BoundPendingNodeSceneEdit<'scene> {
    _seal: BoundSourceSeal,
    pub(super) input: &'scene SceneStorage,
    pub(super) program: NodeSceneEditProgram<'scene>,
    pub(super) operation: OperationIdentity,
    pub(super) counts: NodeSceneEditFixedCounts,
    pub(super) invalidations: CanonicalInvalidations,
}

struct BoundSourceSeal;

impl PendingNodeSceneEdit {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn try_new_node_transform(
        input: &SceneStorage,
        target_source_index: usize,
        target_canonical_row: usize,
        replacement: AuthoredTransform,
        operation: OperationIdentity,
        source_node_rows: usize,
        source_root_rows: usize,
        source_edge_rows: usize,
    ) -> Result<Self, NodeSceneEditPlanError> {
        let target = NodeTransformEditTarget::try_new(target_source_index, target_canonical_row)?;
        let program = NodeSceneEditProgram::node_transform(target, replacement);
        if program.operation_kind() != *operation.kind() {
            return Err(NodeSceneEditPlanError::InvalidSource {
                reason: "sealed edit program disagrees with its operation identity",
            });
        }
        if target.source_index.as_usize() >= source_node_rows {
            return Err(NodeSceneEditPlanError::InvalidSource {
                reason: "target source node index exceeds the proof-qualified source rows",
            });
        }
        let counts = NodeSceneEditFixedCounts::try_new_node_transform(
            input,
            &program,
            source_node_rows,
            source_root_rows,
            source_edge_rows,
        )?;

        Ok(Self {
            input_generation: input.generation(),
            input_root: input.proof.directory(),
            rewrite: NodeTransformRewrite::new(target, replacement),
            operation,
            counts,
            invalidations: CanonicalInvalidations::scene_bounds_invalidated(),
        })
    }

    pub(in crate::canonical_scene::store) const fn input_generation(&self) -> SceneGenerationId {
        self.input_generation
    }

    pub(in crate::canonical_scene::store) const fn invalidations(&self) -> CanonicalInvalidations {
        self.invalidations
    }

    pub(super) fn bind(
        self,
        input: &SceneStorage,
    ) -> Result<BoundPendingNodeSceneEdit<'_>, NodeSceneEditPlanError> {
        if input.generation() != self.input_generation || input.proof.directory() != self.input_root
        {
            return Err(NodeSceneEditPlanError::InputSourceMismatch {
                expected: self.input_generation,
                actual: input.generation(),
            });
        }
        Ok(BoundPendingNodeSceneEdit {
            _seal: BoundSourceSeal,
            input,
            program: NodeSceneEditProgram::NodeTransform(self.rewrite),
            operation: self.operation,
            counts: self.counts,
            invalidations: self.invalidations,
        })
    }
}

impl<'scene> BoundPendingNodeSceneEdit<'scene> {
    pub(super) fn try_triangulation(
        input: &'scene SceneStorage,
        geometry: crate::canonical_scene::store::typed_storage::edit::GeometryTriangulationPlan<
            'scene,
        >,
        options: crate::provenance::OptionsDigest,
        poll: &mut impl FnMut() -> Result<(), NodeSceneEditPlanError>,
    ) -> Result<Self, NodeSceneEditPlanError> {
        let program = NodeSceneEditProgram::TriangulatePrimitive(geometry);
        let proof = input.proof;
        let counts = NodeSceneEditFixedCounts::try_new_for_program(
            input,
            &program,
            proof.source_node_count(),
            proof.source_root_count(),
            proof.source_edge_count(),
            poll,
        )?;
        Ok(Self {
            _seal: BoundSourceSeal,
            input,
            program,
            operation: OperationIdentity::new(OperationKind::Split, options),
            counts,
            invalidations: CanonicalInvalidations::scene_bounds_invalidated(),
        })
    }
}

impl<'scene> BoundPendingNodeSceneEdit<'scene> {
    pub(super) const fn input(&self) -> &'scene SceneStorage {
        self.input
    }

    pub(super) const fn operation(&self) -> &OperationIdentity {
        &self.operation
    }

    pub(super) const fn counts(&self) -> NodeSceneEditFixedCounts {
        self.counts
    }

    pub(super) const fn invalidations(&self) -> CanonicalInvalidations {
        self.invalidations
    }
}

fn checked_add_u32(
    left: u32,
    right: u32,
    quantity: &'static str,
) -> Result<u32, NodeSceneEditPlanError> {
    left.checked_add(right)
        .ok_or(NodeSceneEditPlanError::SizeOverflow { quantity })
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum NodeSceneEditPlanError {
    #[error(
        "node Scene edit plan targets source generation {expected:?}, but execution received a different retained source at generation {actual:?}"
    )]
    InputSourceMismatch {
        expected: SceneGenerationId,
        actual: SceneGenerationId,
    },
    #[error("validated node Scene cannot support the requested edit: {reason}")]
    InvalidSource { reason: &'static str },
    #[error("node Scene edit size overflowed while computing {quantity}")]
    SizeOverflow { quantity: &'static str },
    #[error("node Scene edit could not seal its output validation plan")]
    ValidationPlan(#[from] StorePlanError),
    #[error("Scene edit semantic planning was interrupted")]
    Runtime(#[from] baozi_operation::StoreRuntimeError),
}

fn try_node_transform_profile_counts(
    source: ProfileMeteredCounts,
    source_evidence: SceneEvidenceCounts,
    output_evidence: SceneEvidenceCounts,
    profile: NodeSceneEditProfilePlan,
) -> Result<ProfileMeteredCounts, NodeSceneEditPlanError> {
    if source.resource_count != source_evidence.profile_resource_count
        || source.source_count != source_evidence.profile_source_count
        || source.provenance_count != source_evidence.profile_provenance_count
        || source.operation_count != source_evidence.profile_operation_count
        || source.parent_count != source_evidence.profile_parent_count
        || source.owner_count != source_evidence.profile_owner_count
        || source.attachment_count != source_evidence.profile_attachment_count
        || source.metadata_entry_count != source_evidence.profile_metadata_entry_count
    {
        return Err(NodeSceneEditPlanError::InvalidSource {
            reason: "sealed profile validation counts disagree with source evidence",
        });
    }
    let digest_bytes = u32::try_from(profile.digest.bytes()).map_err(|_| {
        NodeSceneEditPlanError::SizeOverflow {
            quantity: "profile lineage digest bytes",
        }
    })?;
    Ok(ProfileMeteredCounts {
        segment_count: output_evidence.profile_segment_count,
        resource_count: source.resource_count,
        source_count: source.source_count,
        provenance_count: output_evidence.profile_provenance_count,
        operation_count: output_evidence.profile_operation_count,
        evidence_count: source.evidence_count,
        parent_count: output_evidence.profile_parent_count,
        wide_count: source.wide_count,
        lineage_text_bytes: source.lineage_text_bytes,
        lineage_digest_bytes: checked_add_u32(
            source.lineage_digest_bytes,
            digest_bytes,
            "profile lineage digest bytes",
        )?,
        owner_count: output_evidence.profile_owner_count,
        attachment_count: output_evidence.profile_attachment_count,
        metadata_entry_count: output_evidence.profile_metadata_entry_count,
        metadata_text_bytes: source.metadata_text_bytes,
        metadata_payload_bytes: source.metadata_payload_bytes,
        metadata_bool_count: source.metadata_bool_count,
        metadata_i64_count: source.metadata_i64_count,
        metadata_u64_count: source.metadata_u64_count,
        metadata_f64_count: source.metadata_f64_count,
        metadata_f32_count: source.metadata_f32_count,
        canonical_block_count: checked_add_u32(
            source.canonical_block_count,
            NODE_TRANSFORM_PROFILE_CANONICAL_BLOCK_DELTA,
            "profile canonical block count",
        )?,
        payload_leaf_count: checked_add_u32(
            source.payload_leaf_count,
            NODE_TRANSFORM_PROFILE_PAYLOAD_LEAF_DELTA,
            "profile payload leaf count",
        )?,
        shareable_leaf_count: checked_add_u32(
            source.shareable_leaf_count,
            NODE_TRANSFORM_PROFILE_SHAREABLE_LEAF_DELTA,
            "profile shareable leaf count",
        )?,
    })
}

fn require_count(
    actual: usize,
    expected: usize,
    reason: &'static str,
) -> Result<(), NodeSceneEditPlanError> {
    if actual == expected {
        Ok(())
    } else {
        Err(NodeSceneEditPlanError::InvalidSource { reason })
    }
}

#[cfg(test)]
mod tests {
    use super::{NodeSceneEditPlanError, NodeSceneEvidenceLeafPlan, NodeTransformEditTarget};

    #[test]
    fn shareable_leaf_plan_requires_exact_source_and_output_partitions() {
        let exact = NodeSceneEvidenceLeafPlan::try_new(5, 7, 1, 3, 1)
            .expect("exact leaf dispositions must seal");

        assert!(exact.is_exact());
        assert_eq!(exact.inherited.as_usize(), 3);
        assert!(matches!(
            NodeSceneEvidenceLeafPlan::try_new(1, 1, 1, 0, 1),
            Err(NodeSceneEditPlanError::InvalidSource { .. })
        ));
        assert!(matches!(
            NodeSceneEvidenceLeafPlan::try_new(5, 6, 1, 3, 1),
            Err(NodeSceneEditPlanError::InvalidSource { .. })
        ));
    }

    #[test]
    fn target_coordinates_reject_a_non_synthetic_row_offset() {
        let error = NodeTransformEditTarget::try_new(0, 0)
            .expect_err("source index zero cannot address canonical row zero");

        assert!(matches!(
            error,
            NodeSceneEditPlanError::InvalidSource { .. }
        ));
    }
}
