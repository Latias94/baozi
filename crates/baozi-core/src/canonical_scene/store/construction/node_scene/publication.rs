#[cfg(test)]
use baozi_operation::__retained_store_integration::activation::CanonicalFreezePermitIssue;
use baozi_operation::__retained_store_integration::activation::{
    CanonicalFreezePermit, CanonicalJointCommit, CanonicalJointSeal,
    CentrallyRejectedCanonicalStoreSession, FailureEvidenceMember, FailureEvidenceOutcome,
    FailureEvidenceValidationSession, FrozenFailureState, RootedFailureValidation,
    StoreActivationContainment, StoreCanonicalRejection, StoreNeverActivatedCause,
};
#[cfg(test)]
use baozi_operation::__retained_store_integration::authority::PreparedStoreSuccessOutcome;
use baozi_operation::__retained_store_integration::authority::{
    PreparedStoreFailureOutcome, PreparedStoreSuccess,
};
use baozi_operation::{
    ControlCause, FinalProgress, OperationOwner, ProgressPhase, ProgressUnit,
    PublishedCanonicalStore, PublishedOutcomeEvidenceStore, RecordBlock, StoreBuildFailureClass,
    StoreLedgerEvidence, StoreRuntimeError,
};
use thiserror::Error;

use crate::canonical_scene::store::schema::{
    NODE_SCENE_EVIDENCE_FIELD_COUNT, NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE,
    NodeSceneOutcomeEvidenceSchema, SCENE_SCHEMA_REVISION, SceneDirectorySchema,
    node_scene_evidence_field,
};
#[cfg(test)]
use crate::identity::SceneGenerationId;
use crate::identity::{ReservedSceneGeneration, SceneGenerationError};

use super::super::{GENERATION_EXHAUSTED_CODE, SceneConstructionError};
#[cfg(test)]
use super::NodeSceneDraftBuild;
use super::materialization::{
    NodeSceneEvidenceRecipe, NodeSceneLaneOutput, NodeSceneLaneRejection,
    NodeSceneLaneValidationProof, write_node_scene_evidence_row,
};
use super::{
    NODE_SCENE_BUILD_INVARIANT_CODE, NODE_SCENE_STORAGE_REJECTION_CODE,
    NodeSceneConstructionRejection, NodeSceneDraft, PendingNodeSceneFailure,
    PreparedNodeSceneFailure, PreparedNodeSceneFallback, PreparedNodeSceneRichFailure,
    SceneEvidenceCounts, SceneStorage,
};
#[cfg(test)]
use super::{
    NodeSceneConstructionTerminal, NodeSceneFallback, NodeSceneRichFailure, NodeSceneSuccessParts,
    NodeSceneSuccessReport, PendingNodeSceneDraft, PreparedNodeSceneConstruction,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeSceneLaneGenerationRejectionKind {
    MissingDraft,
    DraftMismatch,
    Exhausted,
}

/// Stable generation-admission rejection produced only by core descriptor validation.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error("canonical Scene generation admission rejected the parser descriptor")]
pub struct NodeSceneLaneGenerationRejection {
    kind: NodeSceneLaneGenerationRejectionKind,
}

impl NodeSceneLaneGenerationRejection {
    const fn missing_draft() -> Self {
        Self {
            kind: NodeSceneLaneGenerationRejectionKind::MissingDraft,
        }
    }

    const fn draft_mismatch() -> Self {
        Self {
            kind: NodeSceneLaneGenerationRejectionKind::DraftMismatch,
        }
    }

    const fn exhausted() -> Self {
        Self {
            kind: NodeSceneLaneGenerationRejectionKind::Exhausted,
        }
    }

    pub const fn stable_code(&self) -> u32 {
        match self.kind {
            NodeSceneLaneGenerationRejectionKind::MissingDraft
            | NodeSceneLaneGenerationRejectionKind::DraftMismatch => {
                NODE_SCENE_BUILD_INVARIANT_CODE
            }
            NodeSceneLaneGenerationRejectionKind::Exhausted => GENERATION_EXHAUSTED_CODE,
        }
    }
}

/// Core-owned generation/proof half of a successful joint publication.
///
/// The token carries the actual typed table owners, generation, and proof. It owns no operation,
/// legacy canonical store, ledger, or joint-commit capability.
#[must_use = "the Scene handoff token must consume the matching published canonical owner"]
pub struct NodeSceneHandoffToken {
    generation: ReservedSceneGeneration,
    proof: super::NodeSceneValidationProof,
    tables: crate::canonical_scene::store::typed_storage::NodeGeometryStorage,
}

/// Exact published owners returned only after the prepared canonical root matches core's proof.
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

/// A prepared store did not contain the canonical root proven by the matching core token.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error("prepared canonical Scene owner does not match the validation handoff token")]
pub struct NodeSceneHandoffPairingError;

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

    pub(in crate::canonical_scene::store) fn try_handoff_prepared(
        self,
        store: PreparedStoreSuccess<
            RecordBlock,
            SceneDirectorySchema,
            RecordBlock,
            NodeSceneOutcomeEvidenceSchema,
        >,
    ) -> Result<NodeScenePublishedSuccess, NodeSceneHandoffPairingError> {
        let expected_root = self.proof.directory().into_store();
        let published = store.handoff();
        if published.canonical.root_ref() != expected_root {
            drop(published);
            drop(self);
            return Err(NodeSceneHandoffPairingError);
        }
        Ok(NodeScenePublishedSuccess {
            scene: self.into_scene(published.canonical),
            outcome_evidence: published.outcome_evidence,
            ledger_evidence: published.ledger_evidence,
        })
    }
}

pub type NodeSceneLaneFreezePermit<O, PP, U> = CanonicalFreezePermit<
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

pub type NodeSceneFrozenGenerationRejection<O, PP, U> = CentrallyRejectedCanonicalStoreSession<
    O,
    PP,
    U,
    NodeSceneLaneGenerationRejection,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
    FrozenFailureState,
>;

pub type NodeSceneLaneJointCommit<'joint, O, PP, U> = CanonicalJointCommit<
    'joint,
    O,
    PP,
    U,
    RecordBlock,
    SceneDirectorySchema,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
>;

/// Consumes core's freeze permit without retaining or preparing the operation contribution.
///
/// Missing/foreign drafts and generation exhaustion remain ordinary frozen rejections. On
/// success the HRTB callback receives the owner-free core token together with the matching scoped
/// joint contribution and seal, so neither branded operation capability can escape this call.
#[allow(clippy::type_complexity)]
pub fn consume_node_scene_freeze_permit<O, PP, U, R>(
    permit: NodeSceneLaneFreezePermit<O, PP, U>,
    parser_draft: Option<NodeSceneDraft>,
    reject: impl FnOnce(NodeSceneFrozenGenerationRejection<O, PP, U>) -> R,
    prepare: impl for<'joint> FnOnce(
        NodeSceneHandoffToken,
        NodeSceneLaneJointCommit<'joint, O, PP, U>,
        CanonicalJointSeal<'joint>,
    ) -> R,
) -> Result<R, StoreActivationContainment>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    consume_node_scene_freeze_permit_with_generation_source(
        permit,
        parser_draft,
        crate::identity::SceneGenerationId::reserve,
        reject,
        prepare,
    )
}

#[allow(clippy::type_complexity)]
fn consume_node_scene_freeze_permit_with_generation_source<O, PP, U, R, ReserveGeneration>(
    permit: NodeSceneLaneFreezePermit<O, PP, U>,
    parser_draft: Option<NodeSceneDraft>,
    reserve_generation: ReserveGeneration,
    reject: impl FnOnce(NodeSceneFrozenGenerationRejection<O, PP, U>) -> R,
    prepare: impl for<'joint> FnOnce(
        NodeSceneHandoffToken,
        NodeSceneLaneJointCommit<'joint, O, PP, U>,
        CanonicalJointSeal<'joint>,
    ) -> R,
) -> Result<R, StoreActivationContainment>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    ReserveGeneration: FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
{
    let expected_draft = permit.output().inner().draft;
    let pairing_rejection = match parser_draft {
        None => Some(NodeSceneLaneGenerationRejection::missing_draft()),
        Some(draft) if !node_scene_draft_matches(draft, expected_draft) => {
            Some(NodeSceneLaneGenerationRejection::draft_mismatch())
        }
        Some(_) => None,
    };
    if let Some(cause) = pairing_rejection {
        let stable_code = cause.stable_code();
        return permit.reject_generation(cause, stable_code).map(reject);
    }

    let generation = match reserve_generation() {
        Ok(generation) => generation,
        Err(SceneGenerationError::Exhausted) => {
            let cause = NodeSceneLaneGenerationRejection::exhausted();
            let stable_code = cause.stable_code();
            return permit.reject_generation(cause, stable_code).map(reject);
        }
    };

    Ok(
        permit.bind_joint_commit(|actual_output, proof, joint, seal| {
            prepare(
                NodeSceneHandoffToken {
                    generation,
                    proof: proof.into_inner(),
                    tables: actual_output.into_tables(),
                },
                joint,
                seal,
            )
        }),
    )
}

#[cfg(test)]
macro_rules! prepare_frozen_node_failure {
    ($rooted:expr, $counts:expr) => {{
        let validation = $rooted.validate_rich_failure(|session, root| {
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
fn prepare_node_scene_test_joint<'joint, O, PP, U>(
    handoff: NodeSceneHandoffToken,
    joint: NodeSceneLaneJointCommit<'joint, O, PP, U>,
    seal: CanonicalJointSeal<'joint>,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match joint.prepare(seal)? {
        PreparedStoreSuccessOutcome::Success(store) => Ok(PreparedNodeSceneConstruction::Success {
            token: handoff,
            store,
        }),
        PreparedStoreSuccessOutcome::RichFailure(store) => {
            drop(handoff);
            Ok(PreparedNodeSceneConstruction::from(
                PreparedNodeSceneFailure::RichFailure(PreparedNodeSceneRichFailure { store }),
            ))
        }
        PreparedStoreSuccessOutcome::MinimalFallback(store) => {
            drop(handoff);
            Ok(PreparedNodeSceneConstruction::from(
                PreparedNodeSceneFailure::MinimalFallback(PreparedNodeSceneFallback { store }),
            ))
        }
    }
}

#[cfg(test)]
impl<O, PP, U> PendingNodeSceneDraft<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::canonical_scene::store) fn prepare_with_generation_source(
        self,
        draft: NodeSceneDraft,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        prepare_pending_node_scene_draft(self, Some(draft), Some(reserve_generation))
            .map(|(prepared, _)| prepared)
    }
}

#[cfg(test)]
impl NodeSceneDraftBuild {
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
            Self::Draft { draft, owner } => owner
                .prepare_with_generation_source(draft, reserve_generation)
                .map(PreparedNodeSceneConstruction::handoff),
            Self::Failure { prepared, .. } => Ok(prepared.handoff_for_test()),
        }
    }
}

#[cfg(test)]
fn prepare_pending_node_scene_draft<O, PP, U, ReserveGeneration>(
    owner: PendingNodeSceneDraft<O, PP, U>,
    parser_draft: Option<NodeSceneDraft>,
    reserve_generation: Option<ReserveGeneration>,
) -> Result<(PreparedNodeSceneConstruction, Option<ControlCause>), SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    ReserveGeneration: FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
{
    let PendingNodeSceneDraft {
        candidate,
        final_progress,
    } = owner;
    let output = candidate.output().inner();
    let evidence_counts = output.evidence_counts;
    match candidate.try_issue_freeze_permit(final_progress)? {
        CanonicalFreezePermitIssue::Ready(permit) => {
            let reserve_generation = reserve_generation
                .expect("a matching parser draft retains the generation admission callback");
            consume_node_scene_freeze_permit_with_generation_source(
                permit,
                parser_draft,
                reserve_generation,
                |rejected| prepare_frozen_node_failure!(rejected, evidence_counts),
                prepare_node_scene_test_joint,
            )?
            .map(|prepared| (prepared, None))
        }
        CanonicalFreezePermitIssue::Interrupted(interrupted) => {
            let control_cause = interrupted.cause().control_cause().cloned();
            prepare_frozen_node_failure!(interrupted, evidence_counts)
                .map(|prepared| (prepared, control_cause))
        }
        CanonicalFreezePermitIssue::DestinationRejected(rejected) => {
            let validation = rejected.validate_rich_failure(|session, root| {
                validate_node_scene_failure_evidence(session, root, evidence_counts)
            })?;
            prepare_node_ordinary_construction_failure(validation, final_progress)
                .map(|prepared| (prepared, None))
        }
    }
}

fn node_scene_draft_matches(parser: NodeSceneDraft, activated: NodeSceneDraft) -> bool {
    parser.brand == activated.brand && parser.directory == activated.directory
}

pub(super) trait NodeActivationFailureCause {
    fn is_authoring_rejection(&self) -> bool;

    fn control_cause(&self) -> Option<&ControlCause>;

    fn into_authoring_rejection(self) -> Option<NodeSceneConstructionRejection<StoreRuntimeError>>;
}

impl NodeActivationFailureCause for StoreNeverActivatedCause<NodeSceneLaneRejection> {
    fn is_authoring_rejection(&self) -> bool {
        matches!(
            self,
            StoreNeverActivatedCause::Requested(rejection)
                if rejection.is_authoring_rejection()
        )
    }

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

impl NodeActivationFailureCause for StoreCanonicalRejection<NodeSceneLaneRejection> {
    fn is_authoring_rejection(&self) -> bool {
        matches!(
            self,
            StoreCanonicalRejection::Requested(rejection)
                if rejection.is_authoring_rejection()
        )
    }

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

type PreparedNodeSceneFailureParts = (
    PreparedNodeSceneFailure,
    Option<NodeSceneConstructionRejection<StoreRuntimeError>>,
    Option<ControlCause>,
);

impl<O, PP, U, L> PendingNodeSceneFailure<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::canonical_scene::store) fn prepare(
        self,
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
                    owner.validate_rich_failure(|session, root| {
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
                        owner.validate_rich_failure(|session, root| {
                            validate_node_scene_failure_evidence(session, root, evidence_counts)
                        }),
                        final_progress,
                    )?,
                    None,
                )
            }
            Self::PreparedFallback(fallback) => {
                let (store, _cause) = fallback.into_parts();
                return Ok((
                    PreparedNodeSceneFailure::MinimalFallback(PreparedNodeSceneFallback { store }),
                    None,
                    None,
                ));
            }
        };
        Ok((prepared, authoring_rejection, control_cause))
    }
}

fn retain_node_authoring_rejection(
    rejection: NodeSceneConstructionRejection<StoreRuntimeError>,
) -> Option<NodeSceneConstructionRejection<StoreRuntimeError>> {
    match rejection {
        NodeSceneConstructionRejection::Materialization(_) => None,
        rejection => Some(rejection),
    }
}

/// Prepares a rejected or never-activated scoped outcome after its one-shot evidence lane closes.
pub(super) fn prepare_node_activation_failure<O, PP, U, Cause>(
    outcome: Result<
        FailureEvidenceOutcome<O, PP, U, Cause, RecordBlock, NodeSceneOutcomeEvidenceSchema>,
        StoreActivationContainment,
    >,
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
            let validation = rooted.validate_rich_failure(|session, root| {
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
pub(super) fn prepare_node_central_failure<O, PP, U, Cause>(
    validation: Result<
        RootedFailureValidation<O, PP, U, Cause, RecordBlock, NodeSceneOutcomeEvidenceSchema>,
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

fn prepare_node_ordinary_failure_validation<O, PP, U, Cause>(
    validation: RootedFailureValidation<
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
            Self::Success { token, store } => {
                let published = token
                    .try_handoff_prepared(store)
                    .expect("test Scene joint preparation must preserve its proven canonical root");
                let (scene, outcome_evidence, ledger_evidence) = published.into_parts();
                NodeSceneConstructionTerminal::Success(NodeSceneSuccessParts {
                    scene,
                    report: NodeSceneSuccessReport {
                        outcome_evidence,
                        ledger_evidence,
                    },
                })
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
                    fallback: published.fallback,
                    ledger_evidence: published.ledger_evidence,
                })
            }
        }
    }
}

#[cfg(test)]
mod lane_only_tests {
    use super::*;

    #[test]
    fn generation_rejections_keep_descriptor_and_exhaustion_codes_distinct() {
        assert_eq!(
            NodeSceneLaneGenerationRejection::missing_draft().stable_code(),
            NODE_SCENE_BUILD_INVARIANT_CODE,
        );
        assert_eq!(
            NodeSceneLaneGenerationRejection::draft_mismatch().stable_code(),
            NODE_SCENE_BUILD_INVARIANT_CODE,
        );
        assert_eq!(
            NodeSceneLaneGenerationRejection::exhausted().stable_code(),
            GENERATION_EXHAUSTED_CODE,
        );
    }
}
