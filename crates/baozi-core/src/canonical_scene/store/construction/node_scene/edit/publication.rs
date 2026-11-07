use baozi_operation::__retained_store_integration::activation::{
    CanonicalCauseSlot, CanonicalFreezePermitIssue, CanonicalGenerationSlots,
    CanonicalJointSuccess, DynamicRootedFailureValidation, RootedFailureValidation,
};
use baozi_operation::__retained_store_integration::authority::{
    PreparedStoreFailure, PreparedStoreFailureOutcome, PreparedStoreFallback, PreparedStoreSuccess,
};
use baozi_operation::{
    FinalProgress, OperationOwner, ProgressPhase, ProgressUnit, PublishedOutcomeEvidenceStore,
    RecordBlock, StoreLedgerEvidence, StoreRuntimeError,
};

use crate::canonical_scene::store::schema::{NodeSceneOutcomeEvidenceSchema, SceneDirectorySchema};
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};

use super::super::super::{
    GENERATION_EXHAUSTED_CODE, SceneConstructionError, burn_generation_reservation,
};
use super::super::publication::validate_node_scene_failure_evidence;
use super::super::{
    ActivatedNodeSceneCandidate, NODE_SCENE_BUILD_INVARIANT_CODE, NodeSceneFallback,
    NodeSceneRichFailure, PendingNodeSceneFailure, PreparedNodeSceneFailure,
    PreparedNodeSceneFallback, PreparedNodeSceneRichFailure, SceneStorage,
};
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
    tables: crate::canonical_scene::store::typed_storage::SceneTables,
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
    pub(in crate::canonical_scene::store) scene: SceneStorage,
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
    MinimalFallback {
        // The test terminal retains this physical owner until the terminal itself is dropped.
        _owner: NodeSceneFallback,
    },
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) enum NodeSceneEditGenerationRejection {
    Exhausted,
    RemapInvariant,
}

macro_rules! prepare_frozen_edit_failure {
    ($rooted:expr, $failure_fallback:expr, $counts:expr) => {{
        let validation = $rooted.validate_rich_failure($failure_fallback, |session, root| {
            validate_node_scene_failure_evidence(session, root, $counts)
        })?;
        match validation {
            RootedFailureValidation::Valid(validated) => {
                let (prepared, _cause) = validated.try_prepare_frozen()?.into_parts();
                map_prepared_failure(prepared)
            }
            RootedFailureValidation::Rejected(rejected) => {
                let (store, _cause) = rejected.prepare_minimal_fallback()?.into_parts();
                Ok(PreparedNodeSceneEdit::minimal_fallback(
                    PreparedNodeSceneEditMinimalFallback { store },
                ))
            }
            RootedFailureValidation::Fallback(fallback) => {
                let (store, _cause) = fallback.into_parts();
                Ok(PreparedNodeSceneEdit::minimal_fallback(
                    PreparedNodeSceneEditMinimalFallback { store },
                ))
            }
        }
    }};
}

pub(super) fn prepare_edit_candidate<'inv, O, PP, U>(
    candidate: ActivatedNodeSceneCandidate<'inv, O, PP, U>,
    generation_slots: CanonicalGenerationSlots<'inv, NodeSceneEditGenerationRejection>,
    failure_fallback: CanonicalCauseSlot<'inv, StoreRuntimeError>,
    input_generation: SceneGenerationId,
    invalidations: CanonicalInvalidations,
    final_progress: FinalProgress<PP, U>,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let evidence_counts = candidate.output().inner().evidence_counts;
    match candidate.try_issue_freeze_permit(generation_slots.interruption, final_progress)? {
        CanonicalFreezePermitIssue::Ready(permit) => {
            let generation =
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(reserve_generation)) {
                    Ok(Ok(generation)) => generation,
                    Ok(Err(SceneGenerationError::Exhausted)) => {
                        let rejected = permit.reject_generation(
                            generation_slots.rejection,
                            NodeSceneEditGenerationRejection::Exhausted,
                            GENERATION_EXHAUSTED_CODE,
                        )?;
                        return prepare_frozen_edit_failure!(
                            rejected,
                            failure_fallback,
                            evidence_counts
                        );
                    }
                    Err(payload) => {
                        let _ = permit.contain_generation_fault();
                        std::panic::resume_unwind(payload);
                    }
                };
            let remaps =
                match CanonicalRemapSet::try_all_unchanged(input_generation, generation.id()) {
                    Ok(remaps) => remaps,
                    Err(_) => {
                        burn_generation_reservation(generation);
                        let rejected = permit.reject_generation(
                            generation_slots.rejection,
                            NodeSceneEditGenerationRejection::RemapInvariant,
                            NODE_SCENE_BUILD_INVARIANT_CODE,
                        )?;
                        return prepare_frozen_edit_failure!(
                            rejected,
                            failure_fallback,
                            evidence_counts
                        );
                    }
                };
            permit.bind_joint_commit(|joint, seal| {
                let CanonicalJointSuccess {
                    output,
                    proof,
                    store,
                } = joint.prepare(seal)?;
                Ok(PreparedNodeSceneEdit::success(
                    PreparedNodeSceneEditSuccess {
                        generation,
                        store,
                        proof: proof.into_inner(),
                        tables: output.into_tables(),
                        invalidations,
                        remaps,
                    },
                ))
            })
        }
        CanonicalFreezePermitIssue::Interrupted(interrupted) => {
            prepare_frozen_edit_failure!(interrupted, failure_fallback, evidence_counts)
        }
        CanonicalFreezePermitIssue::DestinationRejected(rejected) => {
            let validation =
                rejected.validate_rich_failure(failure_fallback, |session, root| {
                    validate_node_scene_failure_evidence(session, root, evidence_counts)
                })?;
            prepare_ordinary_failure_validation(validation, final_progress)
        }
    }
}

pub(super) fn prepare_pending_edit_failure<'inv, O, PP, U>(
    pending: PendingNodeSceneFailure<'inv, O, PP, U>,
    failure_fallback: CanonicalCauseSlot<'inv, StoreRuntimeError>,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let (prepared, _authoring_rejection, _control_cause) = pending.prepare(failure_fallback)?;
    Ok(map_prepared_node_scene_failure(prepared))
}

fn prepare_ordinary_failure_validation<O, PP, U, Cause>(
    validation: DynamicRootedFailureValidation<
        '_,
        O,
        PP,
        U,
        Cause,
        RecordBlock,
        NodeSceneOutcomeEvidenceSchema,
    >,
    final_progress: FinalProgress<PP, U>,
) -> Result<PreparedNodeSceneEdit, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match validation {
        RootedFailureValidation::Valid(validated) => {
            let (prepared, _cause) = validated.try_prepare(final_progress)?.into_parts();
            map_prepared_failure(prepared)
        }
        RootedFailureValidation::Rejected(rejected) => {
            let (store, _cause) = rejected.prepare_minimal_fallback()?.into_parts();
            Ok(PreparedNodeSceneEdit::minimal_fallback(
                PreparedNodeSceneEditMinimalFallback { store },
            ))
        }
        RootedFailureValidation::Fallback(fallback) => {
            let (store, _cause) = fallback.into_parts();
            Ok(PreparedNodeSceneEdit::minimal_fallback(
                PreparedNodeSceneEditMinimalFallback { store },
            ))
        }
    }
}

fn map_prepared_node_scene_failure(prepared: PreparedNodeSceneFailure) -> PreparedNodeSceneEdit {
    match prepared {
        PreparedNodeSceneFailure::RichFailure(PreparedNodeSceneRichFailure { store }) => {
            PreparedNodeSceneEdit::rich_failure(PreparedNodeSceneEditRichFailure { store })
        }
        PreparedNodeSceneFailure::MinimalFallback(PreparedNodeSceneFallback { store }) => {
            PreparedNodeSceneEdit::minimal_fallback(PreparedNodeSceneEditMinimalFallback { store })
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
                    scene: SceneStorage {
                        generation,
                        tables: prepared.tables,
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
                NodeSceneEditTerminal::MinimalFallback {
                    _owner: NodeSceneFallback {
                        _fallback: published.fallback,
                        ledger_evidence: published.ledger_evidence,
                    },
                }
            }
        }
    }
}

#[cfg(test)]
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
