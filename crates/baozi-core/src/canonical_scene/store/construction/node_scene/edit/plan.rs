use baozi_operation::StorePlanError;
use thiserror::Error;

use crate::canonical_scene::store::schema::{CanonicalBlockRef, SceneDirectorySchema};
use crate::entity::AuthoredTransform;
use crate::identity::SceneGenerationId;
use crate::provenance::{OperationIdentity, OperationKind};

use super::super::scratch::NodeSceneCentralValidationPlan;
use super::super::{SceneEvidenceCounts, SceneStorage};
use super::profile::{NodeSceneEditProfilePlan, ProfileEditTargets};
use super::report::CanonicalInvalidations;
use crate::canonical_scene::store::typed_storage::edit::space::SceneSpacePlan;

/// A count proven to fit both retained-store plans and canonical `u32` fields.
#[derive(Debug, Clone, Copy)]
pub(super) struct CheckedCount {
    usize_value: usize,
    u32_value: u32,
}

impl CheckedCount {
    pub(super) fn try_from_usize(
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

    pub(super) fn try_from_u32(
        value: u32,
        quantity: &'static str,
    ) -> Result<Self, NodeSceneEditPlanError> {
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
    GenerateNormals {
        geometry: crate::canonical_scene::store::typed_storage::edit::normals::GeometryNormalsPlan<
            'scene,
        >,
        smooth: bool,
    },
    ConvertSpace(SceneSpacePlan<'scene>),
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

    pub(super) const fn operation_kind(&self) -> OperationKind {
        match self {
            Self::NodeTransform(_) => OperationKind::Transform,
            Self::TriangulatePrimitive(_) => OperationKind::Split,
            Self::GenerateNormals { .. } => OperationKind::Generated,
            Self::ConvertSpace(_) => OperationKind::Transform,
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
    pub(super) canonical_attribution_slots: CheckedCount,
    pub(super) outcome_evidence_blocks: CheckedCount,
    pub(super) outcome_evidence_attribution_slots: CheckedCount,
    pub(super) canonical_node_rows: CheckedCount,
    pub(super) source_root_rows: CheckedCount,
    pub(super) logical_meshes: CheckedCount,
    pub(super) primitives: CheckedCount,
    pub(super) attributes: CheckedCount,
    pub(super) mesh_instances: CheckedCount,
    pub(super) profile: NodeSceneEditProfilePlan,
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
                ProfileEditTargets::Node(rewrite.target().canonical_row.as_u32())
            }
            NodeSceneEditProgram::TriangulatePrimitive(geometry) => {
                ProfileEditTargets::Triangulation {
                    primitive: geometry.target_row(),
                }
            }
            NodeSceneEditProgram::GenerateNormals { geometry, smooth } => {
                ProfileEditTargets::Normals {
                    primitive: u32::try_from(geometry.target_row())
                        .map_err(|_| StorePlanError::SizeOverflow)?,
                    attributes: geometry.attribute_mapping(),
                    smooth: *smooth,
                }
            }
            NodeSceneEditProgram::ConvertSpace(plan) => {
                ProfileEditTargets::Space(plan.conversion())
            }
        };
        let source_evidence = input.proof.counts();
        // The concrete operation supplies the final domain counts before profile planning.
        let mut evidence = source_evidence;
        if let NodeSceneEditProgram::GenerateNormals { geometry, .. } = program {
            evidence.attribute_count = geometry.output_attribute_count();
        }
        let profile = NodeSceneEditProfilePlan::try_new(input, targets, evidence, poll)?;
        let counts = profile.output_counts;
        evidence.profile_segment_count = counts.segment_count;
        evidence.profile_provenance_count = counts.provenance_count;
        evidence.profile_operation_count = counts.operation_count;
        evidence.profile_parent_count = counts.parent_count;
        evidence.profile_owner_count = counts.owner_count;
        evidence.profile_attachment_count = counts.attachment_count;
        evidence.profile_metadata_entry_count = counts.metadata_entry_count;
        let source_central_validation = input.proof.central_validation_plan();
        let mut central_validation_plan = source_central_validation.try_rebind_profile_edit(
            evidence,
            counts,
            profile.maximum_depth,
        )?;
        let geometry_work = match program {
            NodeSceneEditProgram::NodeTransform(_) => None,
            NodeSceneEditProgram::TriangulatePrimitive(geometry) => {
                Some(geometry.validation_work())
            }
            NodeSceneEditProgram::GenerateNormals { geometry, .. } => {
                Some(geometry.validation_work())
            }
            NodeSceneEditProgram::ConvertSpace(plan) => Some(plan.geometry().validation_work()),
        };
        if let Some(work) = geometry_work {
            central_validation_plan = central_validation_plan.try_rebind_geometry(
                input
                    .read_geometry()
                    .validation_work()
                    .ok_or(StorePlanError::SizeOverflow)?,
                work,
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

        // The sole canonical record is the destination's new count directory.
        // Semantic payload sharing is handled by the typed owners, never record leaves.
        let canonical_attribution_slots = CheckedCount::try_from_usize(
            canonical_blocks.as_usize().checked_add(1).ok_or(
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
            canonical_attribution_slots,
            outcome_evidence_blocks: CheckedCount::try_from_u32(1, "outcome-evidence block count")?,
            outcome_evidence_attribution_slots: CheckedCount::try_from_u32(
                2,
                "outcome-evidence attribution slot count",
            )?,
            canonical_node_rows,
            source_root_rows,
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
            profile,
        })
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
    pub(super) fn try_space_conversion(
        input: &'scene SceneStorage,
        plan: SceneSpacePlan<'scene>,
        options: crate::provenance::OptionsDigest,
        poll: &mut impl FnMut() -> Result<(), NodeSceneEditPlanError>,
    ) -> Result<Self, NodeSceneEditPlanError> {
        if !plan.matches_source(&input.tables) {
            return Err(NodeSceneEditPlanError::InvalidSource {
                reason: "space conversion plan belongs to another Scene root",
            });
        }
        let program = NodeSceneEditProgram::ConvertSpace(plan);
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
            operation: OperationIdentity::new(OperationKind::Transform, options),
            counts,
            invalidations: CanonicalInvalidations::scene_bounds_invalidated(),
        })
    }

    pub(super) fn try_normals(
        input: &'scene SceneStorage,
        geometry: crate::canonical_scene::store::typed_storage::edit::normals::GeometryNormalsPlan<
            'scene,
        >,
        smooth: bool,
        options: crate::provenance::OptionsDigest,
        poll: &mut impl FnMut() -> Result<(), NodeSceneEditPlanError>,
    ) -> Result<Self, NodeSceneEditPlanError> {
        let program = NodeSceneEditProgram::GenerateNormals { geometry, smooth };
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
            operation: OperationIdentity::new(OperationKind::Generated, options),
            counts,
            invalidations: CanonicalInvalidations::scene_bounds_invalidated(),
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
    use super::{NodeSceneEditPlanError, NodeTransformEditTarget};

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
