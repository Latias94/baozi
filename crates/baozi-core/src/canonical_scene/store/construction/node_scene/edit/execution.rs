use baozi_operation::{
    OperationControl, RetainedStoreBatchLimits, RetainedStoreBlockTablePlan,
    RetainedStorePublicationPlan, RetainedStoreWriterPlan, StorePlanError,
};
use thiserror::Error;

use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};

use super::super::super::{
    SceneConstructionError, SceneConstructionLimits, SceneConstructionPhase,
    SceneConstructionStartError, SceneConstructionUnit, try_plan_scene_operation,
};
use super::super::publication::node_scene_failure_code;
use super::super::scratch::NodeSceneValidationScratchOwner;
use super::super::{SceneStoreKeys, SceneStoreOperation};
use super::graph::{NodeSceneRewriteError, try_rewrite_node_scene};
use super::plan::PendingNodeTransformEdit;
use super::publication::{
    PreparedNodeTransformEdit, prepare_edit_candidate, prepare_edit_minimal_from_assembly,
    prepare_edit_minimal_from_open_recovery, prepare_edit_post_evidence_failure,
    prepare_edit_post_evidence_invariant,
};

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum NodeTransformEditPrepareError {
    #[error("node transform edit operation could not start")]
    Start(#[from] SceneConstructionStartError),
    #[error("node transform edit construction failed closed")]
    Construction(#[from] SceneConstructionError),
}

impl PendingNodeTransformEdit<'_> {
    pub(in crate::canonical_scene::store) fn prepare(
        self,
        limits: SceneConstructionLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
    ) -> Result<PreparedNodeTransformEdit, NodeTransformEditPrepareError> {
        self.prepare_with_control_and_generation_source(limits, control, SceneGenerationId::reserve)
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn prepare_with_generation_source(
        self,
        limits: SceneConstructionLimits,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeTransformEdit, NodeTransformEditPrepareError> {
        self.prepare_with_control_and_generation_source(
            limits,
            OperationControl::default(),
            reserve_generation,
        )
    }

    fn prepare_with_control_and_generation_source(
        self,
        limits: SceneConstructionLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeTransformEdit, NodeTransformEditPrepareError> {
        let terminal_attribution_slots = self
            .counts
            .canonical_attribution_slots
            .as_usize()
            .max(self.counts.outcome_evidence_attribution_slots.as_usize());
        let start = try_plan_scene_operation(limits, control, terminal_attribution_slots)?;
        let publication =
            edit_publication_plan(start.keys, &self).map_err(SceneConstructionError::from)?;
        let raw = SceneStoreOperation::try_start_derived(
            start.control,
            start.ledger_plan,
            &self.input.canonical,
            start.start_plan,
        )
        .map_err(SceneConstructionStartError::from)?;
        let mut assembly = match raw.try_publication_assembly(publication) {
            Ok(assembly) => assembly,
            Err(error) => {
                let recovery = error
                    .into_disposition()
                    .map_err(SceneConstructionError::from)?;
                return prepare_edit_minimal_from_open_recovery(recovery).map_err(Into::into);
            }
        };

        let graph = match try_rewrite_node_scene(&mut assembly, &self, start.keys) {
            Ok(graph) => graph,
            Err(NodeSceneRewriteError::BeforeEvidence(cause)) => {
                return prepare_edit_minimal_from_assembly(
                    assembly,
                    node_scene_failure_code(&cause),
                )
                .map_err(Into::into);
            }
            Err(NodeSceneRewriteError::AfterEvidence { evidence, cause }) => {
                return prepare_edit_post_evidence_failure(
                    assembly,
                    evidence,
                    self.counts.evidence,
                    cause,
                )
                .map_err(Into::into);
            }
        };
        if assembly.canonical_len() != self.counts.canonical_blocks.as_usize()
            || assembly.outcome_evidence_len() != self.counts.outcome_evidence_blocks.as_usize()
        {
            return prepare_edit_post_evidence_invariant(
                assembly,
                graph.evidence,
                self.counts.evidence,
            )
            .map_err(Into::into);
        }
        if let Err(cause) = assembly.try_commit_canonical(
            start.keys.dimension,
            u64::from(self.counts.canonical_blocks.as_u32()),
        ) {
            return prepare_edit_post_evidence_failure(
                assembly,
                graph.evidence,
                self.counts.evidence,
                cause,
            )
            .map_err(Into::into);
        }
        let scratch = match NodeSceneValidationScratchOwner::try_new(
            &mut assembly,
            start.keys,
            self.counts.evidence,
            self.input.proof.deformation_uniqueness_mark_count(),
        ) {
            Ok(scratch) => scratch,
            Err(cause) => {
                return prepare_edit_post_evidence_failure(
                    assembly,
                    graph.evidence,
                    self.counts.evidence,
                    cause,
                )
                .map_err(Into::into);
            }
        };

        prepare_edit_candidate(
            assembly,
            graph,
            scratch,
            start.keys,
            self.input.generation(),
            self.counts.evidence,
            self.counts.canonical_blocks.as_u32(),
            self.input.proof.deformation_uniqueness_mark_count(),
            self.invalidations,
            reserve_generation,
        )
        .map_err(Into::into)
    }

    #[cfg(test)]
    pub(super) fn prepare_with_control_and_generation_source_for_test(
        self,
        limits: SceneConstructionLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeTransformEdit, NodeTransformEditPrepareError> {
        self.prepare_with_control_and_generation_source(limits, control, reserve_generation)
    }
}

fn edit_publication_plan(
    keys: SceneStoreKeys,
    pending: &PendingNodeTransformEdit<'_>,
) -> Result<RetainedStorePublicationPlan, StorePlanError> {
    let expected_custom_rewrites = if pending.target.requires_node_shape_rewrite() {
        3
    } else {
        1
    };
    let source_partition = pending
        .counts
        .source_root_rows
        .as_usize()
        .checked_add(pending.counts.source_edge_rows.as_usize())
        .ok_or(StorePlanError::SizeOverflow)?;
    let physical_partition = pending
        .counts
        .new_canonical_blocks
        .as_usize()
        .checked_add(pending.counts.inherited_shareable_leaves.as_usize())
        .ok_or(StorePlanError::SizeOverflow)?;
    if pending.counts.custom_semantic_rewrite_blocks.as_usize() != expected_custom_rewrites
        || source_partition != pending.counts.source_node_rows.as_usize()
        || pending.counts.source_reference_rows.as_usize()
            != pending.counts.source_node_rows.as_usize()
        || physical_partition != pending.counts.canonical_blocks.as_usize()
    {
        return Err(StorePlanError::InvalidBlockTable);
    }
    let canonical_writer =
        edit_writer_plan(keys, pending.counts.canonical_attribution_slots.as_usize())?;
    let evidence_writer = edit_writer_plan(
        keys,
        pending.counts.outcome_evidence_attribution_slots.as_usize(),
    )?;
    let canonical = RetainedStoreBlockTablePlan::new(
        canonical_writer,
        pending.counts.canonical_blocks.as_usize(),
        pending.counts.new_canonical_blocks.as_usize(),
    )?;
    let evidence = RetainedStoreBlockTablePlan::new(
        evidence_writer,
        pending.counts.outcome_evidence_blocks.as_usize(),
        pending.counts.outcome_evidence_blocks.as_usize(),
    )?;
    Ok(RetainedStorePublicationPlan::success(canonical, evidence))
}

fn edit_writer_plan(
    keys: SceneStoreKeys,
    attribution_slots: usize,
) -> Result<RetainedStoreWriterPlan, StorePlanError> {
    Ok(RetainedStoreWriterPlan::new(
        keys.phase,
        keys.dimension,
        keys.dimension,
        RetainedStoreBatchLimits::new(attribution_slots, 4)?,
    ))
}
