use baozi_operation::__retained_store_integration::activation::{
    CanonicalCauseSlot, CanonicalFreezePermit, CanonicalJointSuccess, FailureEvidenceMember,
    FailureEvidenceValidationSession, RootedFailureValidation, StoreActivationContainment,
    StoreCanonicalRejection,
};
#[cfg(test)]
use baozi_operation::__retained_store_integration::activation::{
    CanonicalFreezePermitIssue, FailureEvidenceOutcome, StoreNeverActivatedCause,
};
use baozi_operation::__retained_store_integration::authority::{
    PreparedStoreFailure, PreparedStoreFailureOutcome, PreparedStoreFallback, PreparedStoreSuccess,
};
use baozi_operation::{
    ControlCause, OperationOwner, ProgressPhase, ProgressUnit, PublishedCanonicalStore,
    PublishedOutcomeEvidenceStore, RecordBlock, StoreLedgerEvidence, StorePrepareError,
    StoreRuntimeError,
};
#[cfg(test)]
use baozi_operation::{FinalProgress, StoreBuildFailureClass};
use thiserror::Error;

use crate::canonical_scene::store::schema::{
    NODE_SCENE_EVIDENCE_FIELD_COUNT, NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE,
    NodeSceneOutcomeEvidenceSchema, SCENE_SCHEMA_REVISION, SceneDirectorySchema,
    node_scene_evidence_field,
};
#[cfg(test)]
use crate::identity::SceneGenerationId;
use crate::identity::{ReservedSceneGeneration, SceneGenerationError};

use super::super::GENERATION_EXHAUSTED_CODE;
#[cfg(test)]
use super::super::SceneConstructionError;
#[cfg(test)]
use super::NodeSceneDraftBuild;
use super::materialization::{
    NodeSceneEvidenceRecipe, NodeSceneLaneOutput, NodeSceneLaneValidationProof,
};
#[cfg(test)]
use super::materialization::{NodeSceneLaneRejection, write_node_scene_evidence_row};
use super::{NODE_SCENE_BUILD_INVARIANT_CODE, SceneEvidenceCounts, SceneStorage};
#[cfg(test)]
use super::{
    NODE_SCENE_STORAGE_REJECTION_CODE, NodeSceneConstructionRejection, PendingNodeSceneFailure,
    PreparedNodeSceneFailure, PreparedNodeSceneFallback, PreparedNodeSceneRichFailure,
};
#[cfg(test)]
use super::{
    NodeSceneConstructionTerminal, NodeSceneFallback, NodeSceneRichFailure, NodeSceneSuccessParts,
    NodeSceneSuccessReport, PendingNodeSceneDraft, PreparedNodeSceneConstruction,
};

/// Host-selected parser disposition for the pending canonical candidate.
///
/// Acceptance is separate from submission and grants no publication authority. Rejection closes
/// the candidate through the existing frozen failure path; the host retains its original cause.
#[derive(Debug, PartialEq, Eq)]
pub enum SceneParseOutcome {
    /// The parser and scope completed successfully after candidate submission.
    Accepted,
    /// The parser or scope failed after candidate submission.
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeSceneLaneGenerationRejectionKind {
    ParseRejected,
    Exhausted,
}

/// Stable rejection of parser acceptance or core generation admission.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error("canonical Scene parser acceptance or generation admission rejected the candidate")]
pub struct NodeSceneLaneGenerationRejection {
    kind: NodeSceneLaneGenerationRejectionKind,
}

impl NodeSceneLaneGenerationRejection {
    const fn parse_rejected() -> Self {
        Self {
            kind: NodeSceneLaneGenerationRejectionKind::ParseRejected,
        }
    }

    const fn exhausted() -> Self {
        Self {
            kind: NodeSceneLaneGenerationRejectionKind::Exhausted,
        }
    }

    pub const fn stable_code(&self) -> u32 {
        match self.kind {
            NodeSceneLaneGenerationRejectionKind::ParseRejected => NODE_SCENE_BUILD_INVARIANT_CODE,
            NodeSceneLaneGenerationRejectionKind::Exhausted => GENERATION_EXHAUSTED_CODE,
        }
    }

    pub const fn is_parse_rejected(&self) -> bool {
        matches!(
            self.kind,
            NodeSceneLaneGenerationRejectionKind::ParseRejected
        )
    }
}

struct NodeSceneHandoffToken {
    generation: ReservedSceneGeneration,
    proof: super::NodeSceneValidationProof,
    tables: crate::canonical_scene::store::typed_storage::SceneTables,
}

/// Exact published owners whose canonical and evidence roots were admitted before generation.
#[must_use = "the published Scene success must be transferred to the import terminal owner"]
pub struct NodeScenePublishedSuccess {
    scene: SceneStorage,
    outcome_evidence: PublishedOutcomeEvidenceStore<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
    ledger_evidence: StoreLedgerEvidence,
}

impl NodeScenePublishedSuccess {
    pub(in crate::canonical_scene::store) fn into_parts(
        self,
    ) -> (
        SceneStorage,
        PublishedOutcomeEvidenceStore<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
        StoreLedgerEvidence,
    ) {
        (self.scene, self.outcome_evidence, self.ledger_evidence)
    }
}

impl NodeSceneHandoffToken {
    fn into_scene(
        self,
        canonical: PublishedCanonicalStore<RecordBlock, SceneDirectorySchema>,
    ) -> SceneStorage {
        SceneStorage {
            generation: self.generation.into_id(),
            tables: self.tables,
            canonical,
            proof: self.proof,
            #[cfg(test)]
            semantic_read_fault: None,
        }
    }

    fn handoff(
        self,
        store: PreparedStoreSuccess<
            RecordBlock,
            SceneDirectorySchema,
            RecordBlock,
            NodeSceneOutcomeEvidenceSchema,
        >,
    ) -> NodeScenePublishedSuccess {
        let published = store.handoff();
        NodeScenePublishedSuccess {
            scene: self.into_scene(published.canonical),
            outcome_evidence: published.outcome_evidence,
            ledger_evidence: published.ledger_evidence,
        }
    }
}

/// Inseparable core proof, generation, tables, and their exact prepared physical owner.
#[must_use = "the prepared Scene success must be handed off or deliberately dropped"]
pub struct PreparedNodeSceneSuccess {
    token: NodeSceneHandoffToken,
    store: PreparedStoreSuccess<
        RecordBlock,
        SceneDirectorySchema,
        RecordBlock,
        NodeSceneOutcomeEvidenceSchema,
    >,
}

impl PreparedNodeSceneSuccess {
    pub(in crate::canonical_scene::store) fn handoff_node_scene(self) -> NodeScenePublishedSuccess {
        self.token.handoff(self.store)
    }
}

/// Closed preparation outcome from the actual candidate's frozen publication boundary.
#[must_use = "the prepared Scene publication owns its exact terminal storage"]
#[allow(
    clippy::large_enum_variant,
    reason = "terminal owners remain inline without an unmetered allocation"
)]
pub enum PreparedNodeScenePublication {
    Success(PreparedNodeSceneSuccess),
    RichFailure {
        store: PreparedStoreFailure<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
        control_cause: Option<ControlCause>,
        rejection: Option<NodeSceneLaneGenerationRejection>,
    },
    MinimalFallback {
        store: PreparedStoreFallback,
        control_cause: Option<ControlCause>,
        rejection: Option<NodeSceneLaneGenerationRejection>,
    },
}

/// A failure to admit or prepare the frozen Scene before its ownership handoff.
#[derive(Debug, Error)]
pub enum NodeScenePublicationError {
    #[error(transparent)]
    Containment(StoreActivationContainment),
    #[error(transparent)]
    Preparation(StorePrepareError),
}

pub type NodeSceneLaneFreezePermit<'inv, O, PP, U> = CanonicalFreezePermit<
    'inv,
    O,
    PP,
    U,
    NodeSceneLaneOutput,
    NodeSceneLaneValidationProof,
    RecordBlock,
    SceneDirectorySchema,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
>;

/// Admits recovery and both proven roots, then prepares one fixed owned Scene outcome.
pub fn consume_node_scene_freeze_permit<'inv, O, PP, U>(
    permit: NodeSceneLaneFreezePermit<'inv, O, PP, U>,
    rejection_slot: CanonicalCauseSlot<
        'inv,
        StoreCanonicalRejection<NodeSceneLaneGenerationRejection>,
    >,
    failure_fallback: CanonicalCauseSlot<'inv, StoreRuntimeError>,
    evidence: NodeSceneEvidenceRecipe,
    parse_outcome: SceneParseOutcome,
) -> Result<PreparedNodeScenePublication, NodeScenePublicationError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    consume_node_scene_freeze_permit_with_generation_source(
        permit,
        rejection_slot,
        failure_fallback,
        evidence,
        parse_outcome,
        crate::identity::SceneGenerationId::reserve,
    )
}

fn consume_node_scene_freeze_permit_with_generation_source<'inv, O, PP, U>(
    permit: NodeSceneLaneFreezePermit<'inv, O, PP, U>,
    rejection_slot: CanonicalCauseSlot<
        'inv,
        StoreCanonicalRejection<NodeSceneLaneGenerationRejection>,
    >,
    failure_fallback: CanonicalCauseSlot<'inv, StoreRuntimeError>,
    evidence: NodeSceneEvidenceRecipe,
    parse_outcome: SceneParseOutcome,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedNodeScenePublication, NodeScenePublicationError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let (permit, (reserve_generation, (), ())) = permit
        .admit_generation_rejection(&rejection_slot, (reserve_generation, (), ()))
        .map_err(NodeScenePublicationError::Containment)?;
    let canonical_root = permit.proof().scene.directory().into_store();
    let evidence_root = permit.proof().evidence_root;
    let (permit, reserve_generation) = permit
        .admit_generation_pairing(canonical_root, evidence_root, reserve_generation)
        .map_err(NodeScenePublicationError::Containment)?;
    if parse_outcome == SceneParseOutcome::Rejected {
        if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            drop(reserve_generation);
        })) {
            let _ = permit.contain_generation_fault();
            std::panic::resume_unwind(payload);
        }
        return prepare_generation_rejection(
            permit,
            rejection_slot,
            failure_fallback,
            evidence,
            NodeSceneLaneGenerationRejection::parse_rejected(),
        );
    }

    let generation =
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(reserve_generation)) {
            Ok(Ok(generation)) => generation,
            Ok(Err(SceneGenerationError::Exhausted)) => {
                return prepare_generation_rejection(
                    permit,
                    rejection_slot,
                    failure_fallback,
                    evidence,
                    NodeSceneLaneGenerationRejection::exhausted(),
                );
            }
            Err(payload) => {
                let _ = permit.contain_generation_fault();
                std::panic::resume_unwind(payload);
            }
        };

    permit.bind_joint_commit(|joint, seal| {
        let CanonicalJointSuccess {
            output,
            proof,
            store,
        } = joint
            .prepare(seal)
            .map_err(NodeScenePublicationError::Containment)?;
        let token = NodeSceneHandoffToken {
            generation,
            proof: proof.into_inner(),
            tables: output.into_tables(),
        };
        Ok(PreparedNodeScenePublication::Success(
            PreparedNodeSceneSuccess { token, store },
        ))
    })
}

#[inline(never)]
fn prepare_generation_rejection<'inv, O, PP, U>(
    permit: NodeSceneLaneFreezePermit<'inv, O, PP, U>,
    rejection_slot: CanonicalCauseSlot<
        'inv,
        StoreCanonicalRejection<NodeSceneLaneGenerationRejection>,
    >,
    failure_fallback: CanonicalCauseSlot<'inv, StoreRuntimeError>,
    evidence: NodeSceneEvidenceRecipe,
    rejection: NodeSceneLaneGenerationRejection,
) -> Result<PreparedNodeScenePublication, NodeScenePublicationError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let rejected = permit
        .reject_generation(rejection_slot, rejection, rejection.stable_code())
        .map_err(NodeScenePublicationError::Containment)?;
    let control_cause = match rejected.cause() {
        StoreCanonicalRejection::Runtime(StoreRuntimeError::Control(cause)) => Some(cause.clone()),
        _ => None,
    };
    let validation = rejected
        .validate_rich_failure(failure_fallback, |session, root| {
            evidence.validate_failure(session, root)
        })
        .map_err(NodeScenePublicationError::Containment)?;
    let (prepared, control_cause) = match validation {
        RootedFailureValidation::Valid(validated) => {
            let (prepared, _cause) = validated
                .try_prepare_frozen()
                .map_err(NodeScenePublicationError::Preparation)?
                .into_parts();
            (prepared, control_cause)
        }
        RootedFailureValidation::Rejected(rejected) => {
            let (store, _cause) = rejected
                .prepare_minimal_fallback()
                .map_err(NodeScenePublicationError::Preparation)?
                .into_parts();
            (
                PreparedStoreFailureOutcome::MinimalFallback(store),
                control_cause,
            )
        }
        RootedFailureValidation::Fallback(fallback) => {
            let control_cause = match fallback.cause() {
                StoreRuntimeError::Control(cause) => Some(cause.clone()),
                _ => control_cause,
            };
            let (store, _cause) = fallback.into_parts();
            (
                PreparedStoreFailureOutcome::MinimalFallback(store),
                control_cause,
            )
        }
    };
    Ok(match prepared {
        PreparedStoreFailureOutcome::RichFailure(store) => {
            PreparedNodeScenePublication::RichFailure {
                store,
                control_cause,
                rejection: Some(rejection),
            }
        }
        PreparedStoreFailureOutcome::MinimalFallback(store) => {
            PreparedNodeScenePublication::MinimalFallback {
                store,
                control_cause,
                rejection: Some(rejection),
            }
        }
    })
}

#[cfg(test)]
macro_rules! prepare_frozen_node_failure {
    ($rooted:expr, $failure_fallback:expr, $counts:expr) => {{
        let validation = $rooted.validate_rich_failure($failure_fallback, |session, root| {
            validate_node_scene_failure_evidence(session, root, $counts)
        })?;
        match validation {
            RootedFailureValidation::Valid(validated) => {
                let (prepared, _cause) = validated.try_prepare_frozen()?.into_parts();
                Ok(PreparedNodeSceneConstruction::from(
                    map_node_prepared_failure(prepared),
                ))
            }
            RootedFailureValidation::Rejected(rejected) => {
                let (store, _cause) = rejected.prepare_minimal_fallback()?.into_parts();
                Ok(PreparedNodeSceneConstruction::from(
                    PreparedNodeSceneFailure::MinimalFallback(PreparedNodeSceneFallback { store }),
                ))
            }
            RootedFailureValidation::Fallback(fallback) => {
                let (store, _cause) = fallback.into_parts();
                Ok(PreparedNodeSceneConstruction::from(
                    PreparedNodeSceneFailure::MinimalFallback(PreparedNodeSceneFallback { store }),
                ))
            }
        }
    }};
}

#[cfg(test)]
impl<O, PP, U> PendingNodeSceneDraft<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::canonical_scene::store) fn prepare_with_generation_source(
        self,
        parse_outcome: SceneParseOutcome,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        prepare_pending_node_scene_draft(self, parse_outcome, reserve_generation)
            .map(|(prepared, _)| prepared)
    }
}

#[cfg(test)]
impl NodeSceneDraftBuild<'_> {
    pub(super) fn from_completion<'inv>(
        completion: baozi_operation::__retained_store_integration::activation::CanonicalInvocationCompletion<
            'inv,
            super::super::SceneConstructionOwner,
            super::super::SceneConstructionPhase,
            super::super::SceneConstructionUnit,
            NodeSceneLaneOutput,
            NodeSceneLaneValidationProof,
            NodeSceneLaneRejection,
            NodeSceneLaneRejection,
            NodeSceneLaneGenerationRejection,
            RecordBlock,
            SceneDirectorySchema,
            RecordBlock,
            NodeSceneOutcomeEvidenceSchema,
        >,
        evidence: NodeSceneEvidenceRecipe,
    ) -> Result<NodeSceneDraftBuild<'inv>, SceneConstructionError> {
        use baozi_operation::__retained_store_integration::activation::CanonicalInvocationOutcome;

        let final_progress = super::super::final_progress();
        let pending = match completion.outcome {
            CanonicalInvocationOutcome::Candidate(candidate) => {
                return Ok(NodeSceneDraftBuild::Draft {
                    owner: PendingNodeSceneDraft {
                        candidate,
                        generation_slots: completion.generation,
                        failure_fallback: completion.failure_fallback,
                        final_progress,
                    },
                });
            }
            CanonicalInvocationOutcome::Rejected(owner) => {
                PendingNodeSceneFailure::ActivationRejected {
                    owner,
                    evidence_plan: evidence.record_plan(),
                    evidence_counts: evidence.counts(),
                    final_progress,
                }
            }
            CanonicalInvocationOutcome::NeverActivated(owner) => {
                PendingNodeSceneFailure::NeverActivated {
                    owner,
                    evidence_plan: evidence.record_plan(),
                    evidence_counts: evidence.counts(),
                    final_progress,
                }
            }
            CanonicalInvocationOutcome::CentralRejected(owner) => {
                PendingNodeSceneFailure::CentralRejected {
                    owner,
                    evidence_counts: evidence.counts(),
                    final_progress,
                }
            }
            CanonicalInvocationOutcome::Interrupted(owner) => {
                PendingNodeSceneFailure::CentralInterrupted {
                    owner,
                    evidence_counts: evidence.counts(),
                    final_progress,
                }
            }
            CanonicalInvocationOutcome::Fallback(owner) => {
                PendingNodeSceneFailure::DeferredFallback(owner)
            }
            CanonicalInvocationOutcome::Operation(_) => {
                unreachable!("Scene activation always records a canonical disposition")
            }
        };
        let (prepared, authoring_rejection, _) = pending.prepare(completion.failure_fallback)?;
        Ok(NodeSceneDraftBuild::Failure {
            authoring_rejection,
            prepared,
        })
    }

    pub(super) fn publish_for_test(
        self,
    ) -> Result<NodeSceneConstructionTerminal, SceneConstructionError> {
        self.publish_with_generation_source_for_test(SceneGenerationId::reserve)
    }

    pub(super) fn publish_with_generation_source_for_test(
        self,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<NodeSceneConstructionTerminal, SceneConstructionError> {
        match self {
            Self::Draft { owner } => owner
                .prepare_with_generation_source(SceneParseOutcome::Accepted, reserve_generation)
                .map(PreparedNodeSceneConstruction::handoff),
            Self::Failure { prepared, .. } => Ok(prepared.handoff_for_test()),
        }
    }
}

#[cfg(test)]
fn prepare_pending_node_scene_draft<O, PP, U, ReserveGeneration>(
    owner: PendingNodeSceneDraft<'_, O, PP, U>,
    parse_outcome: SceneParseOutcome,
    reserve_generation: ReserveGeneration,
) -> Result<(PreparedNodeSceneConstruction, Option<ControlCause>), SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    ReserveGeneration: FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
{
    let PendingNodeSceneDraft {
        candidate,
        generation_slots,
        failure_fallback,
        final_progress,
    } = owner;
    let output = candidate.output().inner();
    let evidence_counts = output.evidence_counts;
    match candidate.try_issue_freeze_permit(generation_slots.interruption, final_progress)? {
        CanonicalFreezePermitIssue::Ready(permit) => {
            let evidence = NodeSceneEvidenceRecipe::from_counts(
                baozi_operation::RecordPlan::new(1, NODE_SCENE_EVIDENCE_FIELD_COUNT)?,
                evidence_counts,
            );
            consume_node_scene_freeze_permit_with_generation_source(
                permit,
                generation_slots.rejection,
                failure_fallback,
                evidence,
                parse_outcome,
                reserve_generation,
            )
            .map(|prepared| (PreparedNodeSceneConstruction::Publication(prepared), None))
            .map_err(|cause| match cause {
                NodeScenePublicationError::Containment(cause) => {
                    SceneConstructionError::Activation(cause)
                }
                NodeScenePublicationError::Preparation(cause) => {
                    SceneConstructionError::Prepare(cause)
                }
            })
        }
        CanonicalFreezePermitIssue::Interrupted(interrupted) => {
            let control_cause = interrupted.cause().control_cause().cloned();
            prepare_frozen_node_failure!(interrupted, failure_fallback, evidence_counts)
                .map(|prepared| (prepared, control_cause))
        }
        CanonicalFreezePermitIssue::DestinationRejected(rejected) => {
            let validation =
                rejected.validate_rich_failure(failure_fallback, |session, root| {
                    validate_node_scene_failure_evidence(session, root, evidence_counts)
                })?;
            prepare_node_ordinary_construction_failure(validation, final_progress)
                .map(|prepared| (prepared, None))
        }
    }
}

#[cfg(test)]
pub(super) trait NodeActivationFailureCause {
    fn control_cause(&self) -> Option<&ControlCause>;

    fn into_authoring_rejection(self) -> Option<NodeSceneConstructionRejection<StoreRuntimeError>>;
}

#[cfg(test)]
impl NodeActivationFailureCause for StoreNeverActivatedCause<NodeSceneLaneRejection> {
    fn control_cause(&self) -> Option<&ControlCause> {
        match self {
            StoreNeverActivatedCause::Runtime(StoreRuntimeError::Control(cause)) => Some(cause),
            StoreNeverActivatedCause::Requested(_) | StoreNeverActivatedCause::Runtime(_) => None,
        }
    }

    fn into_authoring_rejection(self) -> Option<NodeSceneConstructionRejection<StoreRuntimeError>> {
        match self {
            StoreNeverActivatedCause::Requested(rejection) => {
                retain_node_authoring_rejection(rejection.into_inner())
            }
            StoreNeverActivatedCause::Runtime(_) => None,
        }
    }
}

#[cfg(test)]
impl NodeActivationFailureCause for StoreCanonicalRejection<NodeSceneLaneRejection> {
    fn control_cause(&self) -> Option<&ControlCause> {
        match self {
            StoreCanonicalRejection::Runtime(StoreRuntimeError::Control(cause)) => Some(cause),
            StoreCanonicalRejection::Requested(_) | StoreCanonicalRejection::Runtime(_) => None,
        }
    }

    fn into_authoring_rejection(self) -> Option<NodeSceneConstructionRejection<StoreRuntimeError>> {
        match self {
            StoreCanonicalRejection::Requested(rejection) => {
                retain_node_authoring_rejection(rejection.into_inner())
            }
            StoreCanonicalRejection::Runtime(_) => None,
        }
    }
}

#[cfg(test)]
type PreparedNodeSceneFailureParts = (
    PreparedNodeSceneFailure,
    Option<NodeSceneConstructionRejection<StoreRuntimeError>>,
    Option<ControlCause>,
);

#[cfg(test)]
impl<'inv, O, PP, U, L> PendingNodeSceneFailure<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::canonical_scene::store) fn prepare(
        self,
        failure_fallback: CanonicalCauseSlot<'inv, StoreRuntimeError>,
    ) -> Result<PreparedNodeSceneFailureParts, SceneConstructionError> {
        let mut control_cause = None;
        let (prepared, authoring_rejection) = match self {
            Self::ActivationRejected {
                owner,
                evidence_plan,
                evidence_counts,
                final_progress,
            } => {
                control_cause = owner.cause().control_cause().cloned();
                prepare_node_activation_failure(
                    owner.build_failure_evidence(|lane| {
                        match lane.try_retain_records(
                            evidence_plan,
                            NodeSceneOutcomeEvidenceSchema,
                            |writer| {
                                writer.write_row(|row| {
                                    write_node_scene_evidence_row(row, evidence_counts)
                                })
                            },
                        ) {
                            Ok(evidence) => lane.complete(evidence),
                            Err(_) => lane.abort(),
                        }
                    }),
                    failure_fallback,
                    evidence_counts,
                    final_progress,
                )?
            }
            Self::NeverActivated {
                owner,
                evidence_plan,
                evidence_counts,
                final_progress,
            } => {
                control_cause = owner.cause().control_cause().cloned();
                prepare_node_activation_failure(
                    owner.build_failure_evidence(|lane| {
                        match lane.try_retain_records(
                            evidence_plan,
                            NodeSceneOutcomeEvidenceSchema,
                            |writer| {
                                writer.write_row(|row| {
                                    write_node_scene_evidence_row(row, evidence_counts)
                                })
                            },
                        ) {
                            Ok(evidence) => lane.complete(evidence),
                            Err(_) => lane.abort(),
                        }
                    }),
                    failure_fallback,
                    evidence_counts,
                    final_progress,
                )?
            }
            Self::CentralRejected {
                owner,
                evidence_counts,
                final_progress,
            } => (
                prepare_node_central_failure(
                    owner.validate_rich_failure(failure_fallback, |session, root| {
                        validate_node_scene_failure_evidence(session, root, evidence_counts)
                    }),
                    final_progress,
                )?,
                None,
            ),
            Self::CentralInterrupted {
                owner,
                evidence_counts,
                final_progress,
            } => {
                control_cause = owner.cause().control_cause().cloned();
                (
                    prepare_node_central_failure(
                        owner.validate_rich_failure(failure_fallback, |session, root| {
                            validate_node_scene_failure_evidence(session, root, evidence_counts)
                        }),
                        final_progress,
                    )?,
                    None,
                )
            }
            Self::DeferredFallback(fallback) => {
                let control_cause = match fallback.cause() {
                    StoreRuntimeError::Control(cause) => Some(cause.clone()),
                    _ => None,
                };
                let (store, _cause) = fallback.prepare_minimal_fallback()?.into_parts();
                return Ok((
                    PreparedNodeSceneFailure::MinimalFallback(PreparedNodeSceneFallback { store }),
                    None,
                    control_cause,
                ));
            }
        };
        Ok((prepared, authoring_rejection, control_cause))
    }
}

#[cfg(test)]
fn retain_node_authoring_rejection(
    rejection: NodeSceneConstructionRejection<StoreRuntimeError>,
) -> Option<NodeSceneConstructionRejection<StoreRuntimeError>> {
    match rejection {
        NodeSceneConstructionRejection::Materialization(_) => None,
        rejection => Some(rejection),
    }
}

/// Prepares a rejected or never-activated scoped outcome after its one-shot evidence lane closes.
#[cfg(test)]
pub(super) fn prepare_node_activation_failure<'inv, O, PP, U, Cause>(
    outcome: Result<
        FailureEvidenceOutcome<'inv, O, PP, U, Cause, RecordBlock, NodeSceneOutcomeEvidenceSchema>,
        StoreActivationContainment,
    >,
    failure_fallback: CanonicalCauseSlot<'inv, StoreRuntimeError>,
    evidence_counts: SceneEvidenceCounts,
    final_progress: FinalProgress<PP, U>,
) -> Result<
    (
        PreparedNodeSceneFailure,
        Option<NodeSceneConstructionRejection<StoreRuntimeError>>,
    ),
    SceneConstructionError,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Cause: NodeActivationFailureCause,
{
    let (prepared, cause) = match outcome? {
        FailureEvidenceOutcome::Rooted(rooted) => {
            let validation = rooted.validate_rich_failure(failure_fallback, |session, root| {
                validate_node_scene_failure_evidence(session, root, evidence_counts)
            })?;
            prepare_node_ordinary_failure_validation(validation, final_progress)?
        }
        FailureEvidenceOutcome::MinimalFallback(fallback) => {
            let (store, cause) = fallback.into_parts();
            (
                PreparedNodeSceneFailure::MinimalFallback(PreparedNodeSceneFallback { store }),
                Some(cause),
            )
        }
    };
    Ok((
        prepared,
        cause.and_then(NodeActivationFailureCause::into_authoring_rejection),
    ))
}

/// Prepares an ordinary central-validation failure. The caller retains the concrete rejected or
/// interrupted owner long enough to invoke its rich-evidence validator, then passes the typed
/// validation result here without recovering any writable store state.
#[cfg(test)]
pub(super) fn prepare_node_central_failure<O, PP, U, Cause>(
    validation: Result<
        RootedFailureValidation<'_, O, PP, U, Cause, RecordBlock, NodeSceneOutcomeEvidenceSchema>,
        StoreActivationContainment,
    >,
    final_progress: FinalProgress<PP, U>,
) -> Result<PreparedNodeSceneFailure, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let (prepared, _cause) = prepare_node_ordinary_failure_validation(validation?, final_progress)?;
    Ok(prepared)
}

#[cfg(test)]
fn prepare_node_ordinary_construction_failure<O, PP, U, Cause>(
    validation: RootedFailureValidation<
        '_,
        O,
        PP,
        U,
        Cause,
        RecordBlock,
        NodeSceneOutcomeEvidenceSchema,
    >,
    final_progress: FinalProgress<PP, U>,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let (prepared, _cause) = prepare_node_ordinary_failure_validation(validation, final_progress)?;
    Ok(prepared.into())
}

#[cfg(test)]
fn prepare_node_ordinary_failure_validation<O, PP, U, Cause>(
    validation: RootedFailureValidation<
        '_,
        O,
        PP,
        U,
        Cause,
        RecordBlock,
        NodeSceneOutcomeEvidenceSchema,
    >,
    final_progress: FinalProgress<PP, U>,
) -> Result<(PreparedNodeSceneFailure, Option<Cause>), SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match validation {
        RootedFailureValidation::Valid(validated) => {
            let (prepared, cause) = validated.try_prepare(final_progress)?.into_parts();
            Ok((map_node_prepared_failure(prepared), Some(cause)))
        }
        RootedFailureValidation::Rejected(rejected) => {
            let (store, cause) = rejected.prepare_minimal_fallback()?.into_parts();
            Ok((
                PreparedNodeSceneFailure::MinimalFallback(PreparedNodeSceneFallback { store }),
                Some(cause),
            ))
        }
        RootedFailureValidation::Fallback(fallback) => {
            let (store, _cause) = fallback.into_parts();
            Ok((
                PreparedNodeSceneFailure::MinimalFallback(PreparedNodeSceneFallback { store }),
                None,
            ))
        }
    }
}

#[cfg(test)]
fn map_node_prepared_failure(
    prepared: PreparedStoreFailureOutcome<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
) -> PreparedNodeSceneFailure {
    match prepared {
        PreparedStoreFailureOutcome::RichFailure(store) => {
            PreparedNodeSceneFailure::RichFailure(PreparedNodeSceneRichFailure { store })
        }
        PreparedStoreFailureOutcome::MinimalFallback(store) => {
            PreparedNodeSceneFailure::MinimalFallback(PreparedNodeSceneFallback { store })
        }
    }
}

/// Validates all 34 cells of the exact single-record NodeScene failure-evidence contract.
pub(super) fn validate_node_scene_failure_evidence<'session, 'storage, 'control, O, PP, U>(
    session: &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
    root: FailureEvidenceMember<'session, RecordBlock, NodeSceneOutcomeEvidenceSchema>,
    counts: SceneEvidenceCounts,
) -> bool
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let expected = node_scene_evidence_values(counts);
    session
        .visit_records(root, |row_index, row| {
            let mut valid = row_index == 0 && row.fields() == NODE_SCENE_EVIDENCE_FIELD_COUNT;
            for (field, expected) in expected.into_iter().enumerate() {
                let matches = row
                    .scalar::<u32, _>(field, |value| value == expected)
                    .unwrap_or(false);
                valid &= matches;
            }
            valid
        })
        .unwrap_or(false)
}

impl NodeSceneEvidenceRecipe {
    /// Validates rich failure evidence through the import-owned failure-validation session.
    pub fn validate_failure<'session, 'storage, 'control, O, PP, U>(
        &self,
        session: &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
        root: FailureEvidenceMember<'session, RecordBlock, NodeSceneOutcomeEvidenceSchema>,
    ) -> bool
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        validate_node_scene_failure_evidence(session, root, self.counts())
    }
}

fn node_scene_evidence_values(
    counts: SceneEvidenceCounts,
) -> [u32; NODE_SCENE_EVIDENCE_FIELD_COUNT] {
    let mut values = [0; NODE_SCENE_EVIDENCE_FIELD_COUNT];
    values[node_scene_evidence_field::SCHEMA_REVISION] = SCENE_SCHEMA_REVISION;
    values[node_scene_evidence_field::OUTCOME] = NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE;
    values[node_scene_evidence_field::CANONICAL_BLOCK_COUNT] = counts.canonical_block_count;
    values[node_scene_evidence_field::NODE_COUNT] = counts.node_count;
    values[node_scene_evidence_field::CAMERA_COUNT] = counts.camera_count;
    values[node_scene_evidence_field::LIGHT_COUNT] = counts.light_count;
    values[node_scene_evidence_field::IMAGE_COUNT] = counts.image_count;
    values[node_scene_evidence_field::SAMPLER_COUNT] = counts.sampler_count;
    values[node_scene_evidence_field::TEXTURE_COUNT] = counts.texture_count;
    values[node_scene_evidence_field::MATERIAL_COUNT] = counts.material_count;
    values[node_scene_evidence_field::MATERIAL_BINDING_COUNT] = counts.material_binding_count;
    values[node_scene_evidence_field::LOGICAL_MESH_COUNT] = counts.logical_mesh_count;
    values[node_scene_evidence_field::PRIMITIVE_COUNT] = counts.primitive_count;
    values[node_scene_evidence_field::ATTRIBUTE_COUNT] = counts.attribute_count;
    values[node_scene_evidence_field::MESH_INSTANCE_COUNT] = counts.mesh_instance_count;
    values[node_scene_evidence_field::SKIN_COUNT] = counts.skin_count;
    values[node_scene_evidence_field::LOGICAL_MORPH_COUNT] = counts.logical_morph_count;
    values[node_scene_evidence_field::MORPH_DEFINITION_COUNT] = counts.morph_definition_count;
    values[node_scene_evidence_field::PRIMITIVE_DEFORMATION_COUNT] =
        counts.primitive_deformation_count;
    values[node_scene_evidence_field::MORPH_TARGET_COUNT] = counts.morph_target_count;
    values[node_scene_evidence_field::MORPH_DELTA_COUNT] = counts.morph_delta_count;
    values[node_scene_evidence_field::ANIMATION_CLIP_COUNT] = counts.animation_clip_count;
    values[node_scene_evidence_field::ANIMATION_CHANNEL_COUNT] = counts.animation_channel_count;
    values[node_scene_evidence_field::PAYLOAD_BLOCK_COUNT] = counts.payload_block_count;
    values[node_scene_evidence_field::PROFILE_SEGMENT_COUNT] = counts.profile_segment_count;
    values[node_scene_evidence_field::PROFILE_RESOURCE_COUNT] = counts.profile_resource_count;
    values[node_scene_evidence_field::PROFILE_SOURCE_COUNT] = counts.profile_source_count;
    values[node_scene_evidence_field::PROFILE_PROVENANCE_COUNT] = counts.profile_provenance_count;
    values[node_scene_evidence_field::PROFILE_OPERATION_COUNT] = counts.profile_operation_count;
    values[node_scene_evidence_field::PROFILE_PARENT_COUNT] = counts.profile_parent_count;
    values[node_scene_evidence_field::PROFILE_OWNER_COUNT] = counts.profile_owner_count;
    values[node_scene_evidence_field::PROFILE_ATTACHMENT_COUNT] = counts.profile_attachment_count;
    values[node_scene_evidence_field::PROFILE_METADATA_ENTRY_COUNT] =
        counts.profile_metadata_entry_count;
    values[node_scene_evidence_field::SHAREABLE_LEAF_BLOCK_COUNT] =
        counts.shareable_leaf_block_count;
    values
}

#[cfg(test)]
pub(super) const fn post_evidence_build_failure_code(class: StoreBuildFailureClass) -> Option<u32> {
    match class {
        StoreBuildFailureClass::AllocationFailed => Some(NODE_SCENE_STORAGE_REJECTION_CODE),
        _ => None,
    }
}

#[cfg(test)]
impl PreparedNodeSceneConstruction {
    pub(in crate::canonical_scene::store) fn handoff(self) -> NodeSceneConstructionTerminal {
        match self {
            Self::Publication(PreparedNodeScenePublication::Success(prepared)) => {
                let published = prepared.handoff_node_scene();
                let (scene, outcome_evidence, ledger_evidence) = published.into_parts();
                NodeSceneConstructionTerminal::Success(NodeSceneSuccessParts {
                    scene,
                    report: NodeSceneSuccessReport {
                        outcome_evidence,
                        ledger_evidence,
                    },
                })
            }
            Self::Publication(PreparedNodeScenePublication::RichFailure { store, .. }) => {
                PreparedNodeSceneFailure::RichFailure(PreparedNodeSceneRichFailure { store })
                    .handoff_for_test()
            }
            Self::Publication(PreparedNodeScenePublication::MinimalFallback { store, .. }) => {
                PreparedNodeSceneFailure::MinimalFallback(PreparedNodeSceneFallback { store })
                    .handoff_for_test()
            }
            Self::Failure(prepared) => prepared.handoff_for_test(),
        }
    }
}

#[cfg(test)]
impl PreparedNodeSceneFailure {
    pub(in crate::canonical_scene::store) fn handoff_for_test(
        self,
    ) -> NodeSceneConstructionTerminal {
        match self {
            Self::RichFailure(prepared) => {
                let published = prepared.store.handoff();
                NodeSceneConstructionTerminal::RichFailure(NodeSceneRichFailure {
                    outcome_evidence: published.outcome_evidence,
                    ledger_evidence: published.ledger_evidence,
                })
            }
            Self::MinimalFallback(prepared) => {
                let published = prepared.store.handoff();
                NodeSceneConstructionTerminal::MinimalFallback(NodeSceneFallback {
                    _fallback: published.fallback,
                    ledger_evidence: published.ledger_evidence,
                })
            }
        }
    }
}

#[cfg(test)]
mod lane_only_tests {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, Ordering};

    use baozi_operation::__retained_store_integration::activation::{
        CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalCentralValidationOutcome,
        CanonicalValidationDecision, CanonicalValidationProgress,
    };
    use baozi_operation::{
        AccountPoisonReason, OperationControl, PhysicalStorageAccount,
        StorePreparationFailureClass, StoreTerminalCause,
    };

    use crate::canonical_scene::store::construction::node_scene::materialization::{
        NodeSceneActivationRequest, NodeSceneSessionActivationBootstrap,
        node_scene_activation_roles, test_scene_authoring_batch,
    };
    use crate::canonical_scene::store::construction::node_scene::scratch::ScopedNodeSceneValidationScratchOwner;
    use crate::canonical_scene::store::construction::node_scene::tests::{
        single_trs_batch, test_account, test_factory, test_limits,
    };
    use crate::canonical_scene::store::construction::{
        STORE_DIMENSION, SceneCanonicalInvocationHost, SceneConstructionPhase,
        SceneConstructionUnit, SceneFailureEvidenceWork, SceneStoreOperation, final_progress,
        try_plan_scene_operation,
    };
    use crate::canonical_scene::store::production::scene_input_terminal_attribution_slots;
    use crate::canonical_scene::store::validation::validate_node_scene_canonical_graph_metered;

    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Pairing {
        Matched,
        ForeignRecoverySlot,
        ForeignCanonicalRoot,
        ForeignEvidenceRoot,
    }

    struct GenerationCapture<'a> {
        account: &'a PhysicalStorageAccount,
        dropped: &'a Cell<usize>,
        before_containment: &'a Cell<bool>,
        panic_on_drop: bool,
    }

    impl Drop for GenerationCapture<'_> {
        fn drop(&mut self) {
            let snapshot = self.account.snapshot();
            self.before_containment
                .set(!snapshot.poisoned() && snapshot.live_retained_allocations() > 0);
            self.dropped.set(self.dropped.get() + 1);
            assert!(!self.panic_on_drop, "denied generation capture drop panic");
        }
    }

    fn candidate_with_proof<'inv>(
        host: &'inv mut SceneCanonicalInvocationHost,
        account: &PhysicalStorageAccount,
        foreign: Option<(Pairing, NodeSceneLaneValidationProof)>,
    ) -> PendingNodeSceneDraft<'inv> {
        let input = test_scene_authoring_batch(single_trs_batch());
        let limits = crate::SceneValidationLimits::default();
        let attribution_slots = scene_input_terminal_attribution_slots(input, limits).unwrap();
        let transaction = NodeSceneActivationRequest::try_new(input, limits)
            .unwrap()
            .into_transaction();
        let start = try_plan_scene_operation(
            test_limits(),
            OperationControl::default(),
            attribution_slots,
            SceneFailureEvidenceWork::exact_single_record(NODE_SCENE_EVIDENCE_FIELD_COUNT),
        )
        .unwrap();
        let mut inputs = Some((start, transaction));
        let finish = host
            .run_body(|parts, disposition| {
                let (start, mut transaction) = inputs.take().unwrap();
                let operation = SceneStoreOperation::try_start(
                    parts.startup,
                    start.control,
                    start.ledger_plan,
                    account,
                    start.start_plan,
                )
                .unwrap();
                let (writer, failure, _) = NodeSceneSessionActivationBootstrap::try_new()
                    .unwrap()
                    .into_parts();
                let owner = operation
                    .into_session_outcome_owner(
                        node_scene_activation_roles(start.keys).session_policy(),
                        writer,
                        failure,
                    )
                    .unwrap();
                let activated = owner
                    .plan_and_activate_scoped(parts.output, parts.activation, |mut planning| {
                        let (plan, completion) = planning
                            .with_semantic_work(|semantic| transaction.preflight(semantic))
                            .unwrap();
                        match planning.seal_activation(plan, completion) {
                            CanonicalActivationSeal::Sealed(permit) => {
                                permit.activate(|lane, execution| execution.materialize(lane))
                            }
                            CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
                        }
                    })
                    .unwrap();
                let postflight = transaction.finish();
                let CanonicalActivationOutcome::Activated(activated) = activated else {
                    panic!("the publication fixture must materialize its real Scene")
                };
                let recipe = postflight.candidate_evidence(activated.output());
                let central = activated
                    .validate_centrally(
                        parts.proof,
                        parts.validation,
                        CanonicalValidationProgress::new(
                            SceneConstructionPhase::Validation,
                            SceneConstructionUnit::WorkUnits,
                        ),
                        |lane, _| match lane.try_retain_records(
                            recipe.record_plan(),
                            NodeSceneOutcomeEvidenceSchema,
                            |writer| writer.write_row(|row| recipe.write_row(row)),
                        ) {
                            Ok(root) => lane.complete(root),
                            Err(_) => lane.abort(),
                        },
                        |planning, output| {
                            let Some((pairing, foreign_proof)) = foreign else {
                                return postflight.validate_candidate(planning, output);
                            };
                            let output = output.inner();
                            let plan = output.central_validation;
                            let scratch_plan = plan.scratch_plan();
                            planning.validate_with(
                                |checkpoint| {
                                    ScopedNodeSceneValidationScratchOwner::try_allocate(
                                        checkpoint,
                                        scratch_plan,
                                    )
                                },
                                |session, directory, evidence, scratch| {
                                    assert!(scratch.initialize(session, scratch_plan)?);
                                    let scene = validate_node_scene_canonical_graph_metered(
                                        session,
                                        directory,
                                        evidence,
                                        output.evidence_counts,
                                        output.refs,
                                        &output.tables,
                                        plan,
                                        &mut scratch.metered_view(),
                                    )?
                                    .expect(
                                        "the actual fixture must pass metered central validation",
                                    );
                                    let proof = NodeSceneLaneValidationProof {
                                        scene: if pairing == Pairing::ForeignCanonicalRoot {
                                            foreign_proof.scene
                                        } else {
                                            scene
                                        },
                                        evidence_root: if pairing == Pairing::ForeignEvidenceRoot {
                                            foreign_proof.evidence_root
                                        } else {
                                            evidence.reference()
                                        },
                                    };
                                    Ok(CanonicalValidationDecision::accept(proof))
                                },
                            )
                        },
                    )
                    .unwrap();
                let CanonicalCentralValidationOutcome::Candidate(candidate) = central else {
                    panic!("the publication fixture must retain its actual validated candidate")
                };
                candidate.request_finish(disposition).unwrap();
            })
            .unwrap();
        let completion = finish.resume();
        let baozi_operation::__retained_store_integration::activation::CanonicalInvocationOutcome::Candidate(candidate) = completion.outcome else {
            panic!("the publication fixture must restore its validated candidate")
        };
        PendingNodeSceneDraft {
            candidate,
            generation_slots: completion.generation,
            failure_fallback: completion.failure_fallback,
            final_progress: final_progress(),
        }
    }

    fn assert_generation_admission(
        pairing: Pairing,
        parse_rejected: bool,
        exhausted: bool,
        panic_on_drop: bool,
    ) {
        let factory = test_factory();
        let account = test_account(&factory, 1_024);
        let foreign_factory = test_factory();
        let foreign_account = test_account(&foreign_factory, 1_024);
        let mut host = SceneCanonicalInvocationHost::new();
        let mut foreign_host = SceneCanonicalInvocationHost::new();
        let mut recovery_host: SceneCanonicalInvocationHost = SceneCanonicalInvocationHost::new();
        let foreign_owner = candidate_with_proof(&mut foreign_host, &foreign_account, None);
        let foreign_proof = *foreign_owner.candidate.proof();
        let generation_calls = Cell::new(0);
        let capture_dropped = Cell::new(0);
        let capture_before_containment = Cell::new(false);
        let source = AtomicU64::new(if exhausted { u64::MAX } else { 1 });
        let generation_before = source.load(Ordering::Relaxed);
        {
            let foreign = matches!(
                pairing,
                Pairing::ForeignCanonicalRoot | Pairing::ForeignEvidenceRoot
            )
            .then_some((pairing, foreign_proof));
            let owner = candidate_with_proof(&mut host, &account, foreign);
            let expected_root = owner.candidate.output().inner().refs.directory.into_store();
            let expected_evidence_root = owner.candidate.proof().evidence_root;
            let evidence = NodeSceneEvidenceRecipe::from_counts(
                baozi_operation::RecordPlan::new(1, NODE_SCENE_EVIDENCE_FIELD_COUNT).unwrap(),
                owner.candidate.output().inner().evidence_counts,
            );
            let PendingNodeSceneDraft {
                candidate,
                generation_slots,
                failure_fallback,
                final_progress,
            } = owner;
            let CanonicalFreezePermitIssue::Ready(permit) = candidate
                .try_issue_freeze_permit(generation_slots.interruption, final_progress)
                .unwrap()
            else {
                panic!("the actual publication fixture must freeze")
            };
            assert!(permit.destination_counters_are_zero());
            let slot = if pairing == Pairing::ForeignRecoverySlot {
                super::super::super::finish_test_scene_operation(
                    &mut recovery_host,
                    &foreign_account,
                )
                .generation
                .rejection
            } else {
                generation_slots.rejection
            };
            let source_capture = GenerationCapture {
                account: &account,
                dropped: &capture_dropped,
                before_containment: &capture_before_containment,
                panic_on_drop,
            };
            let source_ref = &source;
            let generation_counter = &generation_calls;
            let result = consume_node_scene_freeze_permit_with_generation_source(
                permit,
                slot,
                failure_fallback,
                evidence,
                if parse_rejected {
                    SceneParseOutcome::Rejected
                } else {
                    SceneParseOutcome::Accepted
                },
                move || {
                    let _capture = source_capture;
                    generation_counter.set(generation_counter.get() + 1);
                    SceneGenerationId::reserve_from(source_ref)
                },
            );
            let denied = pairing != Pairing::Matched;
            if denied {
                assert!(matches!(
                    result,
                    Err(NodeScenePublicationError::Containment(_))
                ));
                assert_eq!(generation_calls.get(), 0);
                assert_eq!(source.load(Ordering::Relaxed), generation_before);
                assert_eq!(
                    account.snapshot().first_poison_evidence().unwrap().reason(),
                    AccountPoisonReason::OwnershipGraphUnverified
                );
            } else {
                let failed = parse_rejected || exhausted;
                let ledger = match result.unwrap() {
                    PreparedNodeScenePublication::Success(prepared) if !failed => {
                        let published = prepared.handoff_node_scene();
                        let (scene, evidence, ledger) = published.into_parts();
                        assert_eq!(scene.canonical.root_ref(), expected_root);
                        assert_eq!(scene.proof.directory().into_store(), expected_root);
                        assert_eq!(evidence.root_ref(), expected_evidence_root);
                        assert_eq!(
                            ledger.header().terminal_cause(),
                            StoreTerminalCause::Success
                        );
                        drop((scene, evidence));
                        ledger
                    }
                    PreparedNodeScenePublication::RichFailure {
                        store,
                        control_cause,
                        rejection,
                    } if failed => {
                        let rejection = rejection.expect(
                            "the fixed failure must preserve the precise generation disposition",
                        );
                        assert_eq!(rejection.is_parse_rejected(), parse_rejected);
                        let expected_code = if parse_rejected {
                            NODE_SCENE_BUILD_INVARIANT_CODE
                        } else {
                            GENERATION_EXHAUSTED_CODE
                        };
                        assert_eq!(rejection.stable_code(), expected_code);
                        assert!(control_cause.is_none());
                        let published = store.handoff();
                        let StoreTerminalCause::Failure(cause) =
                            published.ledger_evidence.header().terminal_cause()
                        else {
                            panic!("a parser/generation rejection must retain its failure ledger")
                        };
                        assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
                        assert_eq!(cause.code(), expected_code);
                        assert_eq!(
                            published.outcome_evidence.root_ref(),
                            expected_evidence_root
                        );
                        drop(published.outcome_evidence);
                        published.ledger_evidence
                    }
                    _ => panic!(
                        "normal capacity must preserve the actual success or rich failure owner"
                    ),
                };
                assert_eq!(generation_calls.get(), usize::from(!parse_rejected));
                assert_eq!(
                    source.load(Ordering::Relaxed),
                    if failed {
                        generation_before
                    } else {
                        generation_before + 1
                    }
                );
                let destination = ledger.dimension_by_semantic(STORE_DIMENSION).unwrap();
                assert_eq!(destination.pending_publication(), 0);
                assert_eq!(destination.committed_canonical_output() > 0, !failed);
                assert_eq!(ledger.header().current_live_bytes(), 0);
                assert_eq!(ledger.header().active_reservations(), 0);
                drop(ledger);
            }
            assert_eq!(capture_dropped.get(), 1);
            assert!(capture_before_containment.get());
        }
        drop(foreign_owner);
        drop((host, foreign_host, recovery_host));
        let denied = pairing != Pairing::Matched;
        assert_eq!(account.snapshot().poisoned(), denied);
        assert_eq!(account.snapshot().live_retained_allocations() > 0, denied);
        assert_eq!(account.snapshot().live_retained_bytes() > 0, denied);
        assert!(!foreign_account.snapshot().poisoned());
        assert_eq!(foreign_account.snapshot().live_retained_bytes(), 0);
        assert_eq!(foreign_account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn foreign_generation_recovery_slot_is_denied_before_success_source() {
        for pairing in [Pairing::Matched, Pairing::ForeignRecoverySlot] {
            for parse_rejected in [false, true] {
                assert_generation_admission(pairing, parse_rejected, false, false);
            }
        }
    }

    #[test]
    fn foreign_generation_recovery_slot_is_denied_before_exhausted_source() {
        for pairing in [Pairing::Matched, Pairing::ForeignRecoverySlot] {
            for parse_rejected in [false, true] {
                assert_generation_admission(pairing, parse_rejected, true, false);
            }
        }
    }

    #[test]
    fn foreign_generation_recovery_capture_drop_panic_remains_contained() {
        for parse_rejected in [false, true] {
            for exhausted in [false, true] {
                assert_generation_admission(
                    Pairing::ForeignRecoverySlot,
                    parse_rejected,
                    exhausted,
                    true,
                );
            }
        }
    }

    #[test]
    fn foreign_proof_roots_are_denied_before_generation_or_parser_rejection() {
        for pairing in [Pairing::ForeignCanonicalRoot, Pairing::ForeignEvidenceRoot] {
            for parse_rejected in [false, true] {
                for exhausted in [false, true] {
                    for panic_on_drop in [false, true] {
                        assert_generation_admission(
                            pairing,
                            parse_rejected,
                            exhausted,
                            panic_on_drop,
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn foreign_recovery_multiple_input_panics_remain_contained_at_operation_boundary() {
        for panic_mask in [0b011_u8, 0b101, 0b110, 0b111] {
            let factory = test_factory();
            let account = test_account(&factory, 1_024);
            let mut host = SceneCanonicalInvocationHost::new();
            let mut recovery_host: SceneCanonicalInvocationHost =
                SceneCanonicalInvocationHost::new();
            {
                let owner = candidate_with_proof(&mut host, &account, None);
                let PendingNodeSceneDraft {
                    candidate,
                    generation_slots,
                    final_progress,
                    ..
                } = owner;
                let CanonicalFreezePermitIssue::Ready(permit) = candidate
                    .try_issue_freeze_permit(generation_slots.interruption, final_progress)
                    .unwrap()
                else {
                    panic!("the operation recovery fixture must freeze")
                };
                let capture_dropped: [Cell<usize>; 3] = std::array::from_fn(|_| Cell::new(0));
                let before_containment: [Cell<bool>; 3] = std::array::from_fn(|_| Cell::new(false));
                let calls = Cell::new(0);
                let calls_ref = &calls;
                let [source, rejected, prepared] = std::array::from_fn(|index| GenerationCapture {
                    account: &account,
                    dropped: &capture_dropped[index],
                    before_containment: &before_containment[index],
                    panic_on_drop: panic_mask & (1 << index) != 0,
                });
                let result = permit.admit_generation_rejection(
                    &super::super::super::finish_test_scene_operation(&mut recovery_host, &account)
                        .generation
                        .rejection,
                    (
                        move || {
                            let _capture = source;
                            calls_ref.set(calls_ref.get() + 1);
                        },
                        move || {
                            let _capture = rejected;
                            calls_ref.set(calls_ref.get() + 1);
                        },
                        move || {
                            let _capture = prepared;
                            calls_ref.set(calls_ref.get() + 1);
                        },
                    ),
                );
                assert!(result.is_err());
                assert_eq!(calls.get(), 0);
                assert!(capture_dropped.iter().all(|count| count.get() == 1));
                assert!(before_containment.iter().all(Cell::get));
                drop(result);
            }
            drop((host, recovery_host));
            assert!(account.snapshot().poisoned());
            assert!(account.snapshot().live_retained_allocations() > 0);
            assert!(account.snapshot().live_retained_bytes() > 0);
        }
    }

    #[test]
    fn generation_source_panic_preserves_original_payload_and_contains_candidate() {
        struct OriginalGenerationFault(u32);

        let factory = test_factory();
        let account = test_account(&factory, 1_024);
        let mut host = SceneCanonicalInvocationHost::new();
        let generation_calls = Cell::new(0);
        let capture_dropped = Cell::new(0);
        let before_containment = Cell::new(false);
        let owner = candidate_with_proof(&mut host, &account, None);
        let capture = GenerationCapture {
            account: &account,
            dropped: &capture_dropped,
            before_containment: &before_containment,
            panic_on_drop: false,
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = owner.prepare_with_generation_source(SceneParseOutcome::Accepted, || {
                let _capture = capture;
                generation_calls.set(generation_calls.get() + 1);
                std::panic::panic_any(OriginalGenerationFault(17));
            });
        }));
        let payload = result.expect_err("the generation source panic must resume");
        assert_eq!(
            payload
                .downcast::<OriginalGenerationFault>()
                .ok()
                .unwrap()
                .0,
            17
        );
        assert_eq!(generation_calls.get(), 1);
        assert_eq!(capture_dropped.get(), 1);
        assert!(before_containment.get());
        drop(host);
        assert!(account.snapshot().poisoned());
        assert!(account.snapshot().live_retained_allocations() > 0);
    }

    #[test]
    fn rejected_parser_disposes_unused_panicking_source_before_terminal_work() {
        let factory = test_factory();
        let account = test_account(&factory, 1_024);
        let mut host = SceneCanonicalInvocationHost::new();
        let generation_calls = Cell::new(0);
        let capture_dropped = Cell::new(0);
        let before_containment = Cell::new(false);
        let owner = candidate_with_proof(&mut host, &account, None);
        let capture = GenerationCapture {
            account: &account,
            dropped: &capture_dropped,
            before_containment: &before_containment,
            panic_on_drop: true,
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = owner.prepare_with_generation_source(SceneParseOutcome::Rejected, || {
                let _capture = capture;
                generation_calls.set(generation_calls.get() + 1);
                unreachable!("a rejected parser cannot invoke the generation source")
            });
        }));
        assert!(result.is_err());
        assert_eq!(generation_calls.get(), 0);
        assert_eq!(capture_dropped.get(), 1);
        assert!(before_containment.get());
        drop(host);
        assert!(account.snapshot().poisoned());
        assert!(account.snapshot().live_retained_allocations() > 0);
    }

    #[test]
    fn invalid_rich_failure_recipe_uses_the_existing_minimal_fallback() {
        for parse_rejected in [false, true] {
            let factory = test_factory();
            let account = test_account(&factory, 1_024);
            let mut host = SceneCanonicalInvocationHost::new();
            {
                let owner = candidate_with_proof(&mut host, &account, None);
                let (_, _, unsubmitted_recipe) = NodeSceneSessionActivationBootstrap::try_new()
                    .unwrap()
                    .into_parts();
                let PendingNodeSceneDraft {
                    candidate,
                    generation_slots,
                    failure_fallback,
                    final_progress,
                } = owner;
                let CanonicalFreezePermitIssue::Ready(permit) = candidate
                    .try_issue_freeze_permit(generation_slots.interruption, final_progress)
                    .unwrap()
                else {
                    panic!("the fallback fixture must freeze")
                };
                let generation_calls = Cell::new(0);
                let prepared = consume_node_scene_freeze_permit_with_generation_source(
                    permit,
                    generation_slots.rejection,
                    failure_fallback,
                    unsubmitted_recipe,
                    if parse_rejected {
                        SceneParseOutcome::Rejected
                    } else {
                        SceneParseOutcome::Accepted
                    },
                    || {
                        generation_calls.set(generation_calls.get() + 1);
                        Err(SceneGenerationError::Exhausted)
                    },
                )
                .unwrap();
                let PreparedNodeScenePublication::MinimalFallback {
                    store, rejection, ..
                } = prepared
                else {
                    panic!(
                        "an invalid rich recipe must preserve the existing minimal fallback contract"
                    )
                };
                assert_eq!(rejection.unwrap().is_parse_rejected(), parse_rejected);
                assert_eq!(generation_calls.get(), usize::from(!parse_rejected));
                let published = store.handoff();
                assert!(matches!(
                    published.ledger_evidence.header().terminal_cause(),
                    StoreTerminalCause::Failure(_)
                ));
                assert_eq!(
                    published
                        .ledger_evidence
                        .dimension_by_semantic(STORE_DIMENSION)
                        .unwrap()
                        .committed_canonical_output(),
                    0
                );
                drop(published);
            }
            drop(host);
            assert!(!account.snapshot().poisoned());
            assert_eq!(account.snapshot().live_retained_bytes(), 0);
            assert_eq!(account.snapshot().live_retained_allocations(), 0);
        }
    }

    #[test]
    fn generation_rejections_keep_parse_and_exhaustion_codes_distinct() {
        assert_eq!(
            NodeSceneLaneGenerationRejection::parse_rejected().stable_code(),
            NODE_SCENE_BUILD_INVARIANT_CODE,
        );
        assert_eq!(
            NodeSceneLaneGenerationRejection::exhausted().stable_code(),
            GENERATION_EXHAUSTED_CODE,
        );
    }
}
