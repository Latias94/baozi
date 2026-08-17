use baozi_operation::{
    CanonicalInheritanceSource, PreparedStoreFailure, PreparedStoreFailureOutcome,
    PreparedStoreFallback, PreparedStoreSuccess, PreparedStoreSuccessOutcome,
    PublishedOutcomeEvidenceStore, RecordBlock, StoreControlledSuccessValidation,
    StoreFailureValidation, StoreLedgerEvidence, StoreRuntimeError, StoreWorkError,
};

use crate::canonical_scene::store::schema::{NodeSceneOutcomeEvidenceSchema, SceneDirectorySchema};
use crate::canonical_scene::store::validation::{
    validate_node_scene_canonical_graph, validate_node_scene_evidence_graph,
};
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};

use super::super::super::{
    GENERATION_EXHAUSTED_CODE, SceneConstructionError, SceneStoreAssembly, SceneStoreKeys,
    SceneStoreOpenRecovery, TracerSceneFallback, burn_generation_reservation, final_progress,
};
use super::super::publication::node_scene_failure_code;
use super::super::scratch::{NodeSceneCentralValidationPlan, NodeSceneValidationScratchOwner};
use super::super::{
    NODE_SCENE_BUILD_INVARIANT_CODE, NODE_SCENE_CONTROL_INTERRUPTION_CODE,
    NODE_SCENE_WORK_INTERRUPTION_CODE, NodeSceneRichFailure, NodeSceneStoreFailureValidation,
    SceneEvidenceCounts, ValidatedNodeScene,
};
use super::graph::RewrittenNodeSceneGraph;
use super::report::{CanonicalInvalidations, CanonicalRemapSet};

type PreparedEditStoreSuccess = PreparedStoreSuccess<
    RecordBlock,
    SceneDirectorySchema,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
>;
type PreparedEditStoreFailure = PreparedStoreFailure<RecordBlock, NodeSceneOutcomeEvidenceSchema>;

pub(super) struct PreparedNodeSceneEditSuccess {
    generation: ReservedSceneGeneration,
    store: PreparedEditStoreSuccess,
    proof: crate::canonical_scene::store::validation::NodeSceneValidationProof,
    invalidations: CanonicalInvalidations,
    remaps: CanonicalRemapSet,
}

pub(super) struct PreparedNodeSceneEditRichFailure {
    store: PreparedEditStoreFailure,
}

pub(super) struct PreparedNodeSceneEditMinimalFallback {
    store: PreparedStoreFallback,
}

#[must_use = "every prepared node Scene edit owns an exact frozen publication"]
pub(in crate::canonical_scene::store) struct PreparedNodeSceneEdit {
    state: PreparedNodeSceneEditState,
}

#[allow(
    clippy::large_enum_variant,
    reason = "prepared publication owners remain inline so terminal handoff is allocation-free"
)]
enum PreparedNodeSceneEditState {
    Success(PreparedNodeSceneEditSuccess),
    RichFailure(PreparedNodeSceneEditRichFailure),
    MinimalFallback(PreparedNodeSceneEditMinimalFallback),
}

pub(in crate::canonical_scene::store) struct NodeSceneEditReport {
    invalidations: CanonicalInvalidations,
    remaps: CanonicalRemapSet,
    outcome_evidence: PublishedOutcomeEvidenceStore<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
    ledger_evidence: StoreLedgerEvidence,
}

pub(in crate::canonical_scene::store) struct NodeSceneEditSuccessParts {
    pub(in crate::canonical_scene::store) scene: ValidatedNodeScene,
    pub(in crate::canonical_scene::store) report: NodeSceneEditReport,
}

#[must_use = "the terminal owns the exact node Scene edit outcome"]
#[allow(
    clippy::large_enum_variant,
    reason = "published Scene and evidence owners remain inline so handoff is a pure ownership move"
)]
pub(in crate::canonical_scene::store) enum NodeSceneEditTerminal {
    Success(NodeSceneEditSuccessParts),
    RichFailure(NodeSceneRichFailure),
    MinimalFallback(TracerSceneFallback),
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_edit_candidate(
    assembly: SceneStoreAssembly<CanonicalInheritanceSource>,
    graph: RewrittenNodeSceneGraph,
    mut scratch: NodeSceneValidationScratchOwner,
    keys: SceneStoreKeys,
    input_generation: SceneGenerationId,
    counts: SceneEvidenceCounts,
    canonical_unique_block_count: u32,
    deformation_uniqueness_mark_count: usize,
    central_validation_plan: NodeSceneCentralValidationPlan,
    invalidations: CanonicalInvalidations,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError> {
    let RewrittenNodeSceneGraph {
        directory,
        evidence,
        mut work,
    } = graph;
    let mut proof = None;
    let validation = {
        let mut scratch_view = scratch.validation_view();
        assembly.validate_success_controlled(
            keys.dimension,
            keys.dimension,
            directory.into_store(),
            evidence,
            |session, directory, evidence, control| {
                proof = validate_node_scene_canonical_graph(
                    session,
                    directory,
                    evidence,
                    counts,
                    canonical_unique_block_count,
                    deformation_uniqueness_mark_count,
                    central_validation_plan,
                    &mut scratch_view,
                    |amount| work.observe(control, amount),
                )?;
                work.finish(control)?;
                Ok(proof.is_some())
            },
        )
    };
    drop(scratch);

    match validation {
        StoreControlledSuccessValidation::InvalidWorkDimension { assembly, cause } => {
            prepare_edit_minimal_from_assembly(
                assembly,
                super::super::publication::node_scene_failure_code(&cause),
            )
        }
        StoreControlledSuccessValidation::Valid(store) => match proof {
            Some(proof) => prepare_validated_edit(
                store,
                proof,
                input_generation,
                invalidations,
                reserve_generation,
            ),
            None => prepare_edit_rich_failure(
                store.downgrade_to_rich_failure(NODE_SCENE_BUILD_INVARIANT_CODE),
                counts,
                NODE_SCENE_BUILD_INVARIANT_CODE,
            ),
        },
        StoreControlledSuccessValidation::Rejected(failure) => {
            prepare_edit_rich_failure(failure, counts, NODE_SCENE_BUILD_INVARIANT_CODE)
        }
        StoreControlledSuccessValidation::Interrupted { failure, cause } => {
            prepare_edit_rich_failure(failure, counts, interruption_code(&cause))
        }
        StoreControlledSuccessValidation::FallbackOnly(fallback) => {
            let store = fallback.prepare_minimal_fallback()?;
            Ok(PreparedNodeSceneEdit::minimal_fallback(
                PreparedNodeSceneEditMinimalFallback { store },
            ))
        }
    }
}

fn prepare_validated_edit(
    store: super::super::ValidatedNodeStoreSuccess,
    proof: crate::canonical_scene::store::validation::NodeSceneValidationProof,
    input_generation: SceneGenerationId,
    invalidations: CanonicalInvalidations,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError> {
    let generation = match reserve_generation() {
        Ok(generation) => generation,
        Err(SceneGenerationError::Exhausted) => {
            return prepare_edit_rich_failure(
                store.downgrade_to_rich_failure(GENERATION_EXHAUSTED_CODE),
                proof.counts(),
                GENERATION_EXHAUSTED_CODE,
            );
        }
    };
    let remaps = match CanonicalRemapSet::try_all_unchanged(input_generation, generation.id()) {
        Ok(remaps) => remaps,
        Err(_) => {
            burn_generation_reservation(generation);
            return prepare_edit_rich_failure(
                store.downgrade_to_rich_failure(NODE_SCENE_BUILD_INVARIANT_CODE),
                proof.counts(),
                NODE_SCENE_BUILD_INVARIANT_CODE,
            );
        }
    };

    match store.try_prepare(final_progress())? {
        PreparedStoreSuccessOutcome::Success(store) => Ok(PreparedNodeSceneEdit::success(
            PreparedNodeSceneEditSuccess {
                generation,
                store,
                proof,
                invalidations,
                remaps,
            },
        )),
        PreparedStoreSuccessOutcome::RichFailure(store) => {
            burn_generation_reservation(generation);
            Ok(PreparedNodeSceneEdit::rich_failure(
                PreparedNodeSceneEditRichFailure { store },
            ))
        }
        PreparedStoreSuccessOutcome::MinimalFallback(store) => {
            burn_generation_reservation(generation);
            Ok(PreparedNodeSceneEdit::minimal_fallback(
                PreparedNodeSceneEditMinimalFallback { store },
            ))
        }
    }
}

fn prepare_edit_rich_failure(
    failure: super::super::NodeSceneStoreFailureAssembly,
    counts: SceneEvidenceCounts,
    cause: u32,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError> {
    let evidence = failure.evidence_root();
    finish_edit_rich_failure_validation(
        failure.validate_rich_failure(evidence, |session, evidence| {
            validate_node_scene_evidence_graph(session, evidence, counts)
        }),
        cause,
    )
}

pub(super) fn prepare_edit_post_evidence_failure<L>(
    assembly: SceneStoreAssembly<L>,
    evidence: baozi_operation::OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    counts: SceneEvidenceCounts,
    cause: StoreRuntimeError,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError> {
    match super::super::publication::post_evidence_failure_code(&cause) {
        Some(cause) => prepare_edit_rich_failure_from_assembly(assembly, evidence, counts, cause),
        None => prepare_edit_minimal_from_assembly(
            assembly,
            super::super::publication::node_scene_failure_code(&cause),
        ),
    }
}

pub(super) fn prepare_edit_post_evidence_invariant<L>(
    assembly: SceneStoreAssembly<L>,
    evidence: baozi_operation::OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    counts: SceneEvidenceCounts,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError> {
    prepare_edit_rich_failure_from_assembly(
        assembly,
        evidence,
        counts,
        NODE_SCENE_BUILD_INVARIANT_CODE,
    )
}

fn prepare_edit_rich_failure_from_assembly<L>(
    assembly: SceneStoreAssembly<L>,
    evidence: baozi_operation::OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    counts: SceneEvidenceCounts,
    cause: u32,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError> {
    finish_edit_rich_failure_validation(
        assembly.validate_rich_failure(evidence, |session, evidence| {
            validate_node_scene_evidence_graph(session, evidence, counts)
        }),
        cause,
    )
}

fn finish_edit_rich_failure_validation(
    validation: NodeSceneStoreFailureValidation,
    cause: u32,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError> {
    match validation {
        StoreFailureValidation::Valid(validated) => {
            map_prepared_failure(validated.try_prepare(cause, final_progress())?)
        }
        StoreFailureValidation::Rejected(rejected) => {
            let store = rejected.prepare_minimal_fallback_with(cause)?;
            Ok(PreparedNodeSceneEdit::minimal_fallback(
                PreparedNodeSceneEditMinimalFallback { store },
            ))
        }
        StoreFailureValidation::FallbackOnly(fallback) => {
            let store = fallback.prepare_minimal_fallback()?;
            Ok(PreparedNodeSceneEdit::minimal_fallback(
                PreparedNodeSceneEditMinimalFallback { store },
            ))
        }
    }
}

fn map_prepared_failure(
    prepared: PreparedStoreFailureOutcome<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError> {
    Ok(match prepared {
        PreparedStoreFailureOutcome::RichFailure(store) => {
            PreparedNodeSceneEdit::rich_failure(PreparedNodeSceneEditRichFailure { store })
        }
        PreparedStoreFailureOutcome::MinimalFallback(store) => {
            PreparedNodeSceneEdit::minimal_fallback(PreparedNodeSceneEditMinimalFallback { store })
        }
    })
}

pub(super) fn prepare_edit_minimal_from_open_recovery<L>(
    recovery: SceneStoreOpenRecovery<L>,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError> {
    let store = recovery.prepare_minimal_fallback_with(node_scene_failure_code)?;
    Ok(PreparedNodeSceneEdit::minimal_fallback(
        PreparedNodeSceneEditMinimalFallback { store },
    ))
}

pub(super) fn prepare_edit_minimal_from_assembly<L>(
    assembly: SceneStoreAssembly<L>,
    cause: u32,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError> {
    let store = assembly.prepare_minimal_fallback_with(cause)?;
    Ok(PreparedNodeSceneEdit::minimal_fallback(
        PreparedNodeSceneEditMinimalFallback { store },
    ))
}

fn interruption_code(cause: &StoreWorkError) -> u32 {
    if cause.control_cause().is_some() {
        NODE_SCENE_CONTROL_INTERRUPTION_CODE
    } else if cause.ledger_cause().is_some() {
        NODE_SCENE_WORK_INTERRUPTION_CODE
    } else {
        NODE_SCENE_BUILD_INVARIANT_CODE
    }
}

impl PreparedNodeSceneEdit {
    fn success(prepared: PreparedNodeSceneEditSuccess) -> Self {
        Self {
            state: PreparedNodeSceneEditState::Success(prepared),
        }
    }

    fn rich_failure(prepared: PreparedNodeSceneEditRichFailure) -> Self {
        Self {
            state: PreparedNodeSceneEditState::RichFailure(prepared),
        }
    }

    fn minimal_fallback(prepared: PreparedNodeSceneEditMinimalFallback) -> Self {
        Self {
            state: PreparedNodeSceneEditState::MinimalFallback(prepared),
        }
    }

    pub(in crate::canonical_scene::store) fn handoff(self) -> NodeSceneEditTerminal {
        match self.state {
            PreparedNodeSceneEditState::Success(prepared) => {
                let generation = prepared.generation.into_id();
                let published = prepared.store.handoff();
                NodeSceneEditTerminal::Success(NodeSceneEditSuccessParts {
                    scene: ValidatedNodeScene {
                        generation,
                        canonical: published.canonical,
                        proof: prepared.proof,
                        #[cfg(test)]
                        semantic_read_fault: None,
                    },
                    report: NodeSceneEditReport {
                        invalidations: prepared.invalidations,
                        remaps: prepared.remaps,
                        outcome_evidence: published.outcome_evidence,
                        ledger_evidence: published.ledger_evidence,
                    },
                })
            }
            PreparedNodeSceneEditState::RichFailure(prepared) => {
                let published = prepared.store.handoff();
                NodeSceneEditTerminal::RichFailure(NodeSceneRichFailure {
                    outcome_evidence: published.outcome_evidence,
                    ledger_evidence: published.ledger_evidence,
                })
            }
            PreparedNodeSceneEditState::MinimalFallback(prepared) => {
                let published = prepared.store.handoff();
                NodeSceneEditTerminal::MinimalFallback(TracerSceneFallback {
                    fallback: published.fallback,
                    ledger_evidence: published.ledger_evidence,
                })
            }
        }
    }
}

impl NodeSceneEditReport {
    pub(in crate::canonical_scene::store) const fn invalidations(&self) -> CanonicalInvalidations {
        self.invalidations
    }

    pub(in crate::canonical_scene::store) const fn remaps(&self) -> &CanonicalRemapSet {
        &self.remaps
    }

    pub(in crate::canonical_scene::store) fn outcome_evidence_block_count(&self) -> usize {
        self.outcome_evidence.len()
    }

    pub(in crate::canonical_scene::store) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        &self.ledger_evidence
    }
}
