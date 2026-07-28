use baozi_operation::{
    CounterClass, LedgerCause, OutcomeEvidenceRecordRef, PreparedStoreFailureOutcome,
    PreparedStoreSuccessOutcome, RecordBlock, RetainedStoreReservation, StoreBuildFailureClass,
    StoreControlledSuccessValidation, StoreFailureValidation, StoreRuntimeError, StoreWorkError,
};

use crate::canonical_scene::store::production::{ValidatedGeometryBatch, ValidatedNodeBatch};
use crate::canonical_scene::store::schema::NodeSceneOutcomeEvidenceSchema;
use crate::canonical_scene::store::validation::{
    validate_node_scene_evidence_graph, validate_node_scene_graph,
};
use crate::identity::{ReservedSceneGeneration, SceneGenerationError};

use super::super::{
    GENERATION_EXHAUSTED_CODE, PreparedTracerSceneFallback, SceneConstructionError,
    SceneConstructionOwner, SceneStoreAssembly, SceneStoreKeys, SceneStoreOperation,
    TracerSceneFallback, burn_generation_reservation, final_progress,
};
use super::{
    NODE_SCENE_BUILD_INVARIANT_CODE, NODE_SCENE_COMMITMENT_REJECTION_CODE,
    NODE_SCENE_CONTROL_INTERRUPTION_CODE, NODE_SCENE_STORAGE_REJECTION_CODE,
    NODE_SCENE_WORK_INTERRUPTION_CODE, NodeSceneCandidate, NodeSceneCandidateOwner,
    NodeSceneConstructionTerminal, NodeSceneRichFailure, NodeSceneStoreFailureAssembly,
    NodeSceneStoreFailureValidation, NodeSceneSuccessParts, NodeSceneSuccessReport,
    NodeSceneWorkMeter, PendingNodeSceneCommitments, PreparedNodeSceneConstruction,
    PreparedNodeSceneRichFailure, PreparedNodeSceneSuccess, SceneEvidenceCounts,
    ValidatedNodeScene, ValidatedNodeSceneCandidate,
};

pub(super) fn prepare_node_scene_candidate<'input>(
    candidate: NodeSceneCandidate,
    nodes: ValidatedNodeBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
    geometry: ValidatedGeometryBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
    mut work: NodeSceneWorkMeter,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    let NodeSceneCandidate {
        owner:
            NodeSceneCandidateOwner {
                mut assembly,
                brand,
                keys,
            },
        draft,
        evidence,
        evidence_counts,
    } = candidate;
    if u32::try_from(assembly.canonical_len()) != Ok(evidence_counts.canonical_block_count)
        || assembly.outcome_evidence_len() != 1
    {
        drop(geometry);
        drop(nodes);
        return prepare_node_rich_failure_from_assembly(
            assembly,
            evidence,
            evidence_counts,
            NODE_SCENE_BUILD_INVARIANT_CODE,
        );
    }
    let pending_commitments = match register_node_scene_canonical_commitment(
        &mut assembly,
        keys,
        evidence_counts.canonical_block_count,
    ) {
        Ok(commitments) => commitments,
        Err(_) => {
            drop(geometry);
            drop(nodes);
            return prepare_node_rich_failure_from_assembly(
                assembly,
                evidence,
                evidence_counts,
                NODE_SCENE_COMMITMENT_REJECTION_CODE,
            );
        }
    };
    let brand_matches = draft.brand == brand;
    let mut proof = None;
    let validation = assembly.validate_success_controlled(
        keys.dimension,
        draft.directory.into_store(),
        evidence,
        |session, directory, evidence, control| {
            proof = if brand_matches {
                validate_node_scene_graph(
                    session,
                    directory,
                    evidence,
                    &nodes,
                    &geometry,
                    |amount| work.observe(control, amount),
                )?
            } else {
                None
            };
            work.finish(control)?;
            Ok(proof.is_some())
        },
    );
    // The central validator is the final reader of the compiled plan and its reservation.
    drop(geometry);
    drop(nodes);
    match validation {
        StoreControlledSuccessValidation::InvalidWorkDimension { assembly, cause } => {
            prepare_node_minimal_fallback_from_assembly(assembly, node_scene_failure_code(&cause))
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
}

pub(super) fn prepare_node_rich_failure_from_assembly(
    assembly: SceneStoreAssembly,
    evidence: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    evidence_counts: SceneEvidenceCounts,
    cause: u32,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
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
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    match validation {
        StoreFailureValidation::Valid(validated) => {
            map_node_prepared_failure(validated.try_prepare(cause, final_progress())?)
        }
        StoreFailureValidation::Rejected(rejected) => {
            let store = rejected.prepare_minimal_fallback_with(cause)?;
            Ok(PreparedNodeSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
        StoreFailureValidation::FallbackOnly(fallback) => {
            let store = fallback.prepare_minimal_fallback()?;
            Ok(PreparedNodeSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
    }
}

fn map_node_prepared_failure(
    prepared: PreparedStoreFailureOutcome<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    Ok(match prepared {
        PreparedStoreFailureOutcome::RichFailure(store) => {
            PreparedNodeSceneConstruction::RichFailure(PreparedNodeSceneRichFailure { store })
        }
        PreparedStoreFailureOutcome::MinimalFallback(store) => {
            PreparedNodeSceneConstruction::MinimalFallback(PreparedTracerSceneFallback { store })
        }
    })
}

pub(super) fn prepare_node_minimal_fallback_from_operation(
    operation: SceneStoreOperation,
    cause: u32,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    let store = operation.prepare_minimal_fallback(cause)?;
    Ok(PreparedNodeSceneConstruction::MinimalFallback(
        PreparedTracerSceneFallback { store },
    ))
}

pub(super) fn prepare_node_minimal_fallback_from_assembly(
    assembly: SceneStoreAssembly,
    cause: u32,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    let store = assembly.prepare_minimal_fallback_with(cause)?;
    Ok(PreparedNodeSceneConstruction::MinimalFallback(
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
