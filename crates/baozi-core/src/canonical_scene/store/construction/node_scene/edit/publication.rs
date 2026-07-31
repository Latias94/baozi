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
    SceneStoreOperation, TracerSceneFallback, burn_generation_reservation, final_progress,
};
use super::super::scratch::NodeSceneValidationScratchOwner;
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

pub(super) struct PreparedNodeTransformEditSuccess {
    generation: ReservedSceneGeneration,
    store: PreparedEditStoreSuccess,
    proof: crate::canonical_scene::store::validation::NodeSceneValidationProof,
    invalidations: CanonicalInvalidations,
    remaps: CanonicalRemapSet,
}

pub(super) struct PreparedNodeTransformEditRichFailure {
    store: PreparedEditStoreFailure,
}

pub(super) struct PreparedNodeTransformEditMinimalFallback {
    store: PreparedStoreFallback,
}

#[must_use = "every prepared node transform edit owns an exact frozen publication"]
pub(in crate::canonical_scene::store) struct PreparedNodeTransformEdit {
    state: PreparedNodeTransformEditState,
}

#[allow(
    clippy::large_enum_variant,
    reason = "prepared publication owners remain inline so terminal handoff is allocation-free"
)]
enum PreparedNodeTransformEditState {
    Success(PreparedNodeTransformEditSuccess),
    RichFailure(PreparedNodeTransformEditRichFailure),
    MinimalFallback(PreparedNodeTransformEditMinimalFallback),
}

pub(in crate::canonical_scene::store) struct NodeTransformEditReport {
    invalidations: CanonicalInvalidations,
    remaps: CanonicalRemapSet,
    outcome_evidence: PublishedOutcomeEvidenceStore<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
    ledger_evidence: StoreLedgerEvidence,
}

pub(in crate::canonical_scene::store) struct NodeTransformEditSuccessParts {
    pub(in crate::canonical_scene::store) scene: ValidatedNodeScene,
    pub(in crate::canonical_scene::store) report: NodeTransformEditReport,
}

#[must_use = "the terminal owns the exact node transform edit outcome"]
#[allow(
    clippy::large_enum_variant,
    reason = "published Scene and evidence owners remain inline so handoff is a pure ownership move"
)]
pub(in crate::canonical_scene::store) enum NodeTransformEditTerminal {
    Success(NodeTransformEditSuccessParts),
    RichFailure(NodeSceneRichFailure),
    MinimalFallback(TracerSceneFallback),
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_edit_candidate<'scene>(
    assembly: SceneStoreAssembly<CanonicalInheritanceSource<'scene>>,
    graph: RewrittenNodeSceneGraph,
    mut scratch: NodeSceneValidationScratchOwner,
    keys: SceneStoreKeys,
    input_generation: SceneGenerationId,
    counts: SceneEvidenceCounts,
    canonical_unique_block_count: u32,
    deformation_uniqueness_mark_count: usize,
    invalidations: CanonicalInvalidations,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedNodeTransformEdit, SceneConstructionError> {
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
            Ok(PreparedNodeTransformEdit::minimal_fallback(
                PreparedNodeTransformEditMinimalFallback { store },
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
) -> Result<PreparedNodeTransformEdit, SceneConstructionError> {
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
        PreparedStoreSuccessOutcome::Success(store) => Ok(PreparedNodeTransformEdit::success(
            PreparedNodeTransformEditSuccess {
                generation,
                store,
                proof,
                invalidations,
                remaps,
            },
        )),
        PreparedStoreSuccessOutcome::RichFailure(store) => {
            burn_generation_reservation(generation);
            Ok(PreparedNodeTransformEdit::rich_failure(
                PreparedNodeTransformEditRichFailure { store },
            ))
        }
        PreparedStoreSuccessOutcome::MinimalFallback(store) => {
            burn_generation_reservation(generation);
            Ok(PreparedNodeTransformEdit::minimal_fallback(
                PreparedNodeTransformEditMinimalFallback { store },
            ))
        }
    }
}

fn prepare_edit_rich_failure(
    failure: super::super::NodeSceneStoreFailureAssembly,
    counts: SceneEvidenceCounts,
    cause: u32,
) -> Result<PreparedNodeTransformEdit, SceneConstructionError> {
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
) -> Result<PreparedNodeTransformEdit, SceneConstructionError> {
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
) -> Result<PreparedNodeTransformEdit, SceneConstructionError> {
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
) -> Result<PreparedNodeTransformEdit, SceneConstructionError> {
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
) -> Result<PreparedNodeTransformEdit, SceneConstructionError> {
    match validation {
        StoreFailureValidation::Valid(validated) => {
            map_prepared_failure(validated.try_prepare(cause, final_progress())?)
        }
        StoreFailureValidation::Rejected(rejected) => {
            let store = rejected.prepare_minimal_fallback_with(cause)?;
            Ok(PreparedNodeTransformEdit::minimal_fallback(
                PreparedNodeTransformEditMinimalFallback { store },
            ))
        }
        StoreFailureValidation::FallbackOnly(fallback) => {
            let store = fallback.prepare_minimal_fallback()?;
            Ok(PreparedNodeTransformEdit::minimal_fallback(
                PreparedNodeTransformEditMinimalFallback { store },
            ))
        }
    }
}

fn map_prepared_failure(
    prepared: PreparedStoreFailureOutcome<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
) -> Result<PreparedNodeTransformEdit, SceneConstructionError> {
    Ok(match prepared {
        PreparedStoreFailureOutcome::RichFailure(store) => {
            PreparedNodeTransformEdit::rich_failure(PreparedNodeTransformEditRichFailure { store })
        }
        PreparedStoreFailureOutcome::MinimalFallback(store) => {
            PreparedNodeTransformEdit::minimal_fallback(PreparedNodeTransformEditMinimalFallback {
                store,
            })
        }
    })
}

pub(super) fn prepare_edit_minimal_from_operation<L>(
    operation: SceneStoreOperation<L>,
    cause: u32,
) -> Result<PreparedNodeTransformEdit, SceneConstructionError> {
    let store = operation.prepare_minimal_fallback(cause)?;
    Ok(PreparedNodeTransformEdit::minimal_fallback(
        PreparedNodeTransformEditMinimalFallback { store },
    ))
}

pub(super) fn prepare_edit_minimal_from_assembly<L>(
    assembly: SceneStoreAssembly<L>,
    cause: u32,
) -> Result<PreparedNodeTransformEdit, SceneConstructionError> {
    let store = assembly.prepare_minimal_fallback_with(cause)?;
    Ok(PreparedNodeTransformEdit::minimal_fallback(
        PreparedNodeTransformEditMinimalFallback { store },
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

impl PreparedNodeTransformEdit {
    fn success(prepared: PreparedNodeTransformEditSuccess) -> Self {
        Self {
            state: PreparedNodeTransformEditState::Success(prepared),
        }
    }

    fn rich_failure(prepared: PreparedNodeTransformEditRichFailure) -> Self {
        Self {
            state: PreparedNodeTransformEditState::RichFailure(prepared),
        }
    }

    fn minimal_fallback(prepared: PreparedNodeTransformEditMinimalFallback) -> Self {
        Self {
            state: PreparedNodeTransformEditState::MinimalFallback(prepared),
        }
    }

    pub(in crate::canonical_scene::store) fn handoff(self) -> NodeTransformEditTerminal {
        match self.state {
            PreparedNodeTransformEditState::Success(prepared) => {
                let generation = prepared.generation.into_id();
                let published = prepared.store.handoff();
                NodeTransformEditTerminal::Success(NodeTransformEditSuccessParts {
                    scene: ValidatedNodeScene {
                        generation,
                        canonical: published.canonical,
                        proof: prepared.proof,
                        #[cfg(test)]
                        semantic_read_fault: None,
                    },
                    report: NodeTransformEditReport {
                        invalidations: prepared.invalidations,
                        remaps: prepared.remaps,
                        outcome_evidence: published.outcome_evidence,
                        ledger_evidence: published.ledger_evidence,
                    },
                })
            }
            PreparedNodeTransformEditState::RichFailure(prepared) => {
                let published = prepared.store.handoff();
                NodeTransformEditTerminal::RichFailure(NodeSceneRichFailure {
                    outcome_evidence: published.outcome_evidence,
                    ledger_evidence: published.ledger_evidence,
                })
            }
            PreparedNodeTransformEditState::MinimalFallback(prepared) => {
                let published = prepared.store.handoff();
                NodeTransformEditTerminal::MinimalFallback(TracerSceneFallback {
                    fallback: published.fallback,
                    ledger_evidence: published.ledger_evidence,
                })
            }
        }
    }
}

impl NodeTransformEditReport {
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
