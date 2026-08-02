use baozi_operation::{
    CounterClass, LedgerCause, OutcomeEvidenceRecordRef, PreparedStoreFailureOutcome,
    PreparedStoreSuccessOutcome, RecordBlock, StoreBuildFailureClass,
    StoreControlledSuccessValidation, StoreFailureValidation, StoreRuntimeError, StoreWorkError,
};

use crate::canonical_scene::store::schema::NodeSceneOutcomeEvidenceSchema;
use crate::canonical_scene::store::validation::{
    validate_node_scene_canonical_graph, validate_node_scene_evidence_graph,
};
use crate::identity::{ReservedSceneGeneration, SceneGenerationError};

use super::super::{
    GENERATION_EXHAUSTED_CODE, PreparedTracerSceneFallback, SceneConstructionError,
    SceneStoreAssembly, SceneStoreKeys, SceneStoreOpenRecovery, SceneStoreOperation,
    TracerSceneFallback, burn_generation_reservation, final_progress,
};
use super::scratch::NodeSceneValidationScratchOwner;
use super::{
    NODE_SCENE_BUILD_INVARIANT_CODE, NODE_SCENE_COMMITMENT_REJECTION_CODE,
    NODE_SCENE_CONTROL_INTERRUPTION_CODE, NODE_SCENE_STORAGE_REJECTION_CODE,
    NODE_SCENE_WORK_INTERRUPTION_CODE, NodeSceneConstructionTerminal, NodeSceneDraft,
    NodeSceneRichFailure, NodeSceneStoreFailureAssembly, NodeSceneStoreFailureValidation,
    NodeSceneSuccessParts, NodeSceneSuccessReport, PendingNodeSceneCommitments,
    PendingNodeSceneDraft, PreparedNodeSceneConstruction, PreparedNodeSceneFailure,
    PreparedNodeSceneRichFailure, PreparedNodeSceneSuccess, SceneEvidenceCounts,
    ValidatedNodeScene, ValidatedNodeSceneCandidate,
};

impl PendingNodeSceneDraft {
    pub(in crate::canonical_scene::store) fn prepare_with_generation_source(
        self,
        draft: NodeSceneDraft,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        prepare_pending_node_scene_draft(self, draft, reserve_generation)
    }

    pub(in crate::canonical_scene::store) fn prepare_missing_draft(
        self,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        let Self {
            assembly,
            evidence,
            evidence_counts,
            ..
        } = self;
        prepare_node_rich_failure_from_assembly(
            assembly,
            evidence,
            evidence_counts,
            NODE_SCENE_BUILD_INVARIANT_CODE,
        )
        .map(Into::into)
    }
}

fn prepare_pending_node_scene_draft(
    owner: PendingNodeSceneDraft,
    draft: NodeSceneDraft,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    let PendingNodeSceneDraft {
        mut assembly,
        brand,
        keys,
        expected_directory,
        evidence,
        evidence_counts,
        mut work,
        deformation_uniqueness_capacity,
    } = owner;
    // CanonicalBlockRef equality includes the destination identity and slot, so this rejects both
    // a foreign construction brand and a same-brand descriptor projected from another store.
    if draft.brand != brand || draft.directory != expected_directory {
        return prepare_node_rich_failure_from_assembly(
            assembly,
            evidence,
            evidence_counts,
            NODE_SCENE_BUILD_INVARIANT_CODE,
        )
        .map(Into::into);
    }
    if u32::try_from(assembly.canonical_len()) != Ok(evidence_counts.canonical_block_count)
        || assembly.outcome_evidence_len() != 1
    {
        return prepare_node_rich_failure_from_assembly(
            assembly,
            evidence,
            evidence_counts,
            NODE_SCENE_BUILD_INVARIANT_CODE,
        )
        .map(Into::into);
    }
    let pending_commitments = match register_node_scene_canonical_commitment(
        &mut assembly,
        keys,
        evidence_counts.canonical_block_count,
    ) {
        Ok(commitments) => commitments,
        Err(_) => {
            return prepare_node_rich_failure_from_assembly(
                assembly,
                evidence,
                evidence_counts,
                NODE_SCENE_COMMITMENT_REJECTION_CODE,
            )
            .map(Into::into);
        }
    };
    let mut scratch = match NodeSceneValidationScratchOwner::try_new(
        &mut assembly,
        keys,
        evidence_counts,
        deformation_uniqueness_capacity,
    ) {
        Ok(scratch) => scratch,
        Err(cause) => {
            return prepare_node_draft_post_evidence_failure(
                assembly,
                evidence,
                evidence_counts,
                cause,
            );
        }
    };
    let mut proof = None;
    let validation = {
        let mut scratch_view = scratch.validation_view();
        assembly.validate_success_controlled(
            keys.dimension,
            draft.directory.into_store(),
            evidence,
            |session, directory, evidence, control| {
                proof = validate_node_scene_canonical_graph(
                    session,
                    directory,
                    evidence,
                    evidence_counts,
                    evidence_counts.canonical_block_count,
                    deformation_uniqueness_capacity,
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
            prepare_node_minimal_fallback_from_assembly(assembly, node_scene_failure_code(&cause))
                .map(Into::into)
        }
        StoreControlledSuccessValidation::Valid(store) => match proof {
            Some(proof) => prepare_validated_node_scene(
                ValidatedNodeSceneCandidate {
                    store,
                    evidence_counts,
                    proof,
                    pending_commitments,
                },
                reserve_generation,
            ),
            None => prepare_node_rich_failure(
                store.downgrade_to_rich_failure(NODE_SCENE_BUILD_INVARIANT_CODE),
                evidence_counts,
                NODE_SCENE_BUILD_INVARIANT_CODE,
            ),
        },
        StoreControlledSuccessValidation::Rejected(failure) => {
            prepare_node_rich_failure(failure, evidence_counts, NODE_SCENE_BUILD_INVARIANT_CODE)
        }
        StoreControlledSuccessValidation::Interrupted { failure, cause } => {
            prepare_node_rich_failure(
                failure,
                evidence_counts,
                controlled_interruption_code(&cause),
            )
        }
        StoreControlledSuccessValidation::FallbackOnly(fallback) => {
            let store = fallback.prepare_minimal_fallback()?;
            Ok(PreparedNodeSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
    }
}

pub(super) fn post_evidence_failure_code(cause: &StoreRuntimeError) -> Option<u32> {
    match cause {
        StoreRuntimeError::Control(_) => Some(NODE_SCENE_CONTROL_INTERRUPTION_CODE),
        StoreRuntimeError::Build(cause) => post_evidence_build_failure_code(cause.class()),
        StoreRuntimeError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::Attempted | CounterClass::Consumed,
                ..
            }
            | LedgerCause::ArithmeticOverflow {
                class: CounterClass::Attempted | CounterClass::Consumed,
                ..
            }
            | LedgerCause::ConsumedExceedsAttempted { .. },
        ) => Some(NODE_SCENE_WORK_INTERRUPTION_CODE),
        StoreRuntimeError::Ledger(
            LedgerCause::LimitExceeded {
                class:
                    CounterClass::Live
                    | CounterClass::PendingPublication
                    | CounterClass::Committed
                    | CounterClass::PublishedOutcomeEvidence,
                ..
            }
            | LedgerCause::ArithmeticOverflow {
                class:
                    CounterClass::Live
                    | CounterClass::PendingPublication
                    | CounterClass::Committed
                    | CounterClass::PublishedOutcomeEvidence,
                ..
            },
        )
        | StoreRuntimeError::BatchAllocationFailed
        | StoreRuntimeError::PhysicalByteLimit { .. }
        | StoreRuntimeError::PhysicalAllocationLimit { .. }
        | StoreRuntimeError::AllocationFailed
        | StoreRuntimeError::MembershipExhausted => Some(NODE_SCENE_STORAGE_REJECTION_CODE),
        _ => None,
    }
}

pub(super) const fn post_evidence_build_failure_code(class: StoreBuildFailureClass) -> Option<u32> {
    match class {
        StoreBuildFailureClass::AllocationFailed => Some(NODE_SCENE_STORAGE_REJECTION_CODE),
        _ => None,
    }
}

pub(super) fn node_scene_failure_code(cause: &StoreRuntimeError) -> u32 {
    post_evidence_failure_code(cause).unwrap_or(NODE_SCENE_BUILD_INVARIANT_CODE)
}

fn prepare_node_draft_post_evidence_failure(
    assembly: SceneStoreAssembly,
    evidence: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    evidence_counts: SceneEvidenceCounts,
    cause: StoreRuntimeError,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    match post_evidence_failure_code(&cause) {
        Some(cause) => {
            prepare_node_rich_failure_from_assembly(assembly, evidence, evidence_counts, cause)
                .map(Into::into)
        }
        None => {
            prepare_node_minimal_fallback_from_assembly(assembly, node_scene_failure_code(&cause))
                .map(Into::into)
        }
    }
}

fn controlled_interruption_code(cause: &StoreWorkError) -> u32 {
    if cause.control_cause().is_some() {
        NODE_SCENE_CONTROL_INTERRUPTION_CODE
    } else if cause.ledger_cause().is_some() {
        NODE_SCENE_WORK_INTERRUPTION_CODE
    } else {
        NODE_SCENE_BUILD_INVARIANT_CODE
    }
}

fn register_node_scene_canonical_commitment(
    assembly: &mut SceneStoreAssembly,
    keys: SceneStoreKeys,
    canonical_block_count: u32,
) -> Result<PendingNodeSceneCommitments, StoreRuntimeError> {
    assembly.try_commit_canonical(keys.dimension, u64::from(canonical_block_count))?;
    Ok(PendingNodeSceneCommitments)
}

fn prepare_validated_node_scene(
    candidate: ValidatedNodeSceneCandidate,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    let ValidatedNodeSceneCandidate {
        store,
        evidence_counts,
        proof,
        pending_commitments,
    } = candidate;
    let generation = match reserve_generation() {
        Ok(generation) => generation,
        Err(SceneGenerationError::Exhausted) => {
            return prepare_node_rich_failure(
                store.downgrade_to_rich_failure(GENERATION_EXHAUSTED_CODE),
                evidence_counts,
                GENERATION_EXHAUSTED_CODE,
            );
        }
    };
    let PendingNodeSceneCommitments = pending_commitments;
    match store.try_prepare(final_progress())? {
        PreparedStoreSuccessOutcome::Success(store) => Ok(PreparedNodeSceneConstruction::Success(
            PreparedNodeSceneSuccess {
                generation,
                store,
                proof,
            },
        )),
        PreparedStoreSuccessOutcome::RichFailure(store) => {
            burn_generation_reservation(generation);
            Ok(PreparedNodeSceneConstruction::RichFailure(
                PreparedNodeSceneRichFailure { store },
            ))
        }
        PreparedStoreSuccessOutcome::MinimalFallback(store) => {
            burn_generation_reservation(generation);
            Ok(PreparedNodeSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
    }
}

fn prepare_node_rich_failure(
    failure: NodeSceneStoreFailureAssembly,
    evidence_counts: SceneEvidenceCounts,
    cause: u32,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    let evidence = failure.evidence_root();
    finish_node_rich_failure_validation(
        failure.validate_rich_failure(evidence, |session, evidence| {
            validate_node_scene_evidence_graph(session, evidence, evidence_counts)
        }),
        cause,
    )
    .map(Into::into)
}

pub(super) fn prepare_node_rich_failure_from_assembly(
    assembly: SceneStoreAssembly,
    evidence: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    evidence_counts: SceneEvidenceCounts,
    cause: u32,
) -> Result<PreparedNodeSceneFailure, SceneConstructionError> {
    finish_node_rich_failure_validation(
        assembly.validate_rich_failure(evidence, |session, evidence| {
            validate_node_scene_evidence_graph(session, evidence, evidence_counts)
        }),
        cause,
    )
}

fn finish_node_rich_failure_validation(
    validation: NodeSceneStoreFailureValidation,
    cause: u32,
) -> Result<PreparedNodeSceneFailure, SceneConstructionError> {
    match validation {
        StoreFailureValidation::Valid(validated) => Ok(map_node_prepared_failure(
            validated.try_prepare(cause, final_progress())?,
        )),
        StoreFailureValidation::Rejected(rejected) => {
            let store = rejected.prepare_minimal_fallback_with(cause)?;
            Ok(PreparedNodeSceneFailure::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
        StoreFailureValidation::FallbackOnly(fallback) => {
            let store = fallback.prepare_minimal_fallback()?;
            Ok(PreparedNodeSceneFailure::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
    }
}

fn map_node_prepared_failure(
    prepared: PreparedStoreFailureOutcome<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
) -> PreparedNodeSceneFailure {
    match prepared {
        PreparedStoreFailureOutcome::RichFailure(store) => {
            PreparedNodeSceneFailure::RichFailure(PreparedNodeSceneRichFailure { store })
        }
        PreparedStoreFailureOutcome::MinimalFallback(store) => {
            PreparedNodeSceneFailure::MinimalFallback(PreparedTracerSceneFallback { store })
        }
    }
}

pub(super) fn prepare_node_minimal_fallback_from_operation(
    operation: SceneStoreOperation,
    cause: u32,
) -> Result<PreparedNodeSceneFailure, SceneConstructionError> {
    let store = operation.prepare_minimal_fallback(cause)?;
    Ok(PreparedNodeSceneFailure::MinimalFallback(
        PreparedTracerSceneFallback { store },
    ))
}

pub(super) fn prepare_node_minimal_fallback_from_open_recovery(
    recovery: SceneStoreOpenRecovery,
) -> Result<PreparedNodeSceneFailure, SceneConstructionError> {
    let store = recovery.prepare_minimal_fallback_with(node_scene_failure_code)?;
    Ok(PreparedNodeSceneFailure::MinimalFallback(
        PreparedTracerSceneFallback { store },
    ))
}

pub(super) fn prepare_node_minimal_fallback_from_assembly(
    assembly: SceneStoreAssembly,
    cause: u32,
) -> Result<PreparedNodeSceneFailure, SceneConstructionError> {
    let store = assembly.prepare_minimal_fallback_with(cause)?;
    Ok(PreparedNodeSceneFailure::MinimalFallback(
        PreparedTracerSceneFallback { store },
    ))
}

impl PreparedNodeSceneConstruction {
    pub(in crate::canonical_scene::store) fn handoff(self) -> NodeSceneConstructionTerminal {
        match self {
            Self::Success(prepared) => {
                let published = prepared.store.handoff();
                NodeSceneConstructionTerminal::Success(NodeSceneSuccessParts {
                    scene: ValidatedNodeScene {
                        generation: prepared.generation.into_id(),
                        canonical: published.canonical,
                        proof: prepared.proof,
                        #[cfg(test)]
                        semantic_read_fault: None,
                    },
                    report: NodeSceneSuccessReport {
                        outcome_evidence: published.outcome_evidence,
                        ledger_evidence: published.ledger_evidence,
                    },
                })
            }
            Self::RichFailure(prepared) => {
                let published = prepared.store.handoff();
                NodeSceneConstructionTerminal::RichFailure(NodeSceneRichFailure {
                    outcome_evidence: published.outcome_evidence,
                    ledger_evidence: published.ledger_evidence,
                })
            }
            Self::MinimalFallback(prepared) => {
                let published = prepared.store.handoff();
                NodeSceneConstructionTerminal::MinimalFallback(TracerSceneFallback {
                    fallback: published.fallback,
                    ledger_evidence: published.ledger_evidence,
                })
            }
        }
    }
}
