//! Hidden cross-crate bridge for the U4 canonical import cutover.

use std::fmt;
use std::num::NonZeroU64;

use baozi_operation::__canonical_activation::{
    CanonicalFailureRequirements, CanonicalSuccessRequirements, CanonicalTableShape,
};
use baozi_operation::{
    OperationOwner, PreparedStoreSuccess, ProgressPhase, ProgressUnit,
    PublishedOutcomeEvidenceStore, RecordBlock, RetainedStoreFailureTerminalWorkRequirement,
    StoreLedgerEvidence,
};

use crate::validation_control::SceneValidationLimits;

use super::super::construction::node_scene::NodeSceneActivationRequest;
use super::super::construction::node_scene::consume_node_scene_freeze_permit;
pub use super::super::construction::node_scene::{
    NodeSceneActivationPolicyRequirements as SceneActivationRequirements,
    NodeSceneActivationPostflight as SceneActivationPostflight,
    NodeSceneActivationTransaction as SceneActivationTransaction,
    NodeSceneEvidenceRecipe as SceneEvidenceRecipe,
    NodeSceneFrozenGenerationRejection as SceneFrozenGenerationRejection,
    NodeSceneHandoffPairingError as SceneHandoffPairingError,
    NodeSceneHandoffToken as SceneHandoffToken, NodeSceneLaneFreezePermit as SceneFreezePermit,
    NodeSceneLaneGenerationRejection as SceneGenerationRejection,
    NodeSceneLaneJointCommit as SceneJointCommit, NodeSceneLaneOutput as SceneActivationOutput,
    NodeSceneLanePlan as SceneSealedActivationPlan,
    NodeSceneLaneRejection as SceneActivationRejection,
    NodeSceneLaneValidationProof as SceneValidationProof,
};
use super::super::construction::{
    FAILURE_TERMINAL_EVIDENCE_MEMBERS, SceneConstructionStartError,
    failure_terminal_work_requirement,
};
use super::super::production::SceneInputEnvelope;
use super::super::schema::NODE_SCENE_EVIDENCE_FIELD_COUNT;
pub use super::super::schema::{
    NodeSceneOutcomeEvidenceSchema as SceneOutcomeEvidenceTag,
    SceneDirectorySchema as SceneCanonicalRootTag,
};
use super::authoring::SceneAuthoringBatch;
use super::scene::ValidatedScene;

pub use super::SceneDraft;

/// Roleless complete-batch request which can be consumed only with one retained operation.
#[must_use = "the Scene activation request must be consumed by one import session owner"]
pub struct SceneActivationRequest<'input>(NodeSceneActivationRequest<'input>);

/// Returns the maximum canonical block count admitted under `limits`.
pub fn maximum_scene_canonical_blocks(limits: SceneValidationLimits) -> Option<usize> {
    SceneInputEnvelope::maximum_canonical_blocks(limits)
}

/// Returns the protected structural-work ceiling for any fresh Scene admitted under `limits`.
pub fn maximum_scene_structural_work(limits: SceneValidationLimits) -> Option<u64> {
    let canonical_blocks = maximum_scene_canonical_blocks(limits)?;
    let canonical = CanonicalTableShape::new(canonical_blocks, canonical_blocks).ok()?;
    let success =
        CanonicalSuccessRequirements::new(canonical, canonical_blocks, NonZeroU64::MIN).ok()?;
    let failure_blocks = FAILURE_TERMINAL_EVIDENCE_MEMBERS;
    let failure = CanonicalFailureRequirements::new(
        CanonicalTableShape::new(failure_blocks, failure_blocks).ok()?,
    );
    success.structural_work_requirement(&failure).ok()
}

impl<'input> SceneActivationRequest<'input> {
    /// Validates the bounded input envelope without acquiring operation or storage authority.
    #[allow(
        clippy::result_large_err,
        reason = "the authority-free request preserves the complete bounded core rejection inline"
    )]
    pub fn try_new(
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
    ) -> Result<Self, SceneActivationRequestError> {
        NodeSceneActivationRequest::try_new(input, validation_limits)
            .map(Self)
            .map_err(SceneActivationRequestError)
    }

    /// Splits authority-free resource requirements from the core transaction payload.
    pub fn into_lane_parts(
        self,
    ) -> (
        SceneActivationRequirements,
        SceneActivationTransaction<'input>,
    ) {
        self.0.into_lane_parts()
    }
}

impl SceneActivationOutput {
    /// Copies the non-owning parser descriptor out of a validated materialization candidate.
    pub fn draft(&self) -> SceneDraft {
        SceneDraft((*self).node_scene_draft())
    }
}

impl SceneHandoffToken {
    /// Verifies and publishes the prepared owner paired with this generation/proof token.
    pub fn handoff(
        self,
        store: PreparedStoreSuccess<
            RecordBlock,
            SceneCanonicalRootTag,
            RecordBlock,
            SceneOutcomeEvidenceTag,
        >,
    ) -> Result<ScenePublishedSuccess, SceneHandoffPairingError> {
        self.try_handoff_prepared(store).map(ScenePublishedSuccess)
    }
}

/// Exact published Scene, outcome evidence, and ledger produced by one paired handoff.
#[must_use = "the published Scene success must be transferred to the import terminal owner"]
pub struct ScenePublishedSuccess(super::super::construction::node_scene::NodeScenePublishedSuccess);

impl ScenePublishedSuccess {
    pub fn into_parts(
        self,
    ) -> (
        ValidatedScene,
        PublishedOutcomeEvidenceStore<RecordBlock, SceneOutcomeEvidenceTag>,
        StoreLedgerEvidence,
    ) {
        let (scene, outcome_evidence, ledger_evidence) = self.0.into_parts();
        (
            ValidatedScene::from_node_scene(scene),
            outcome_evidence,
            ledger_evidence,
        )
    }
}

/// Consumes the freeze permit after core verifies the parser descriptor and admits a generation.
///
/// Core immediately forwards the owner-free handoff token and both scoped joint capabilities to
/// `prepare`; it never stores or prepares the operation contribution.
#[allow(
    clippy::result_large_err,
    clippy::type_complexity,
    reason = "the freeze pivot preserves exact containment and scoped joint ownership without boxing"
)]
pub fn consume_scene_freeze_permit<O, PP, U, R>(
    permit: SceneFreezePermit<O, PP, U>,
    parser_draft: Option<SceneDraft>,
    reject: impl FnOnce(SceneFrozenGenerationRejection<O, PP, U>) -> R,
    prepare: impl for<'joint> FnOnce(
        SceneHandoffToken,
        SceneJointCommit<'joint, O, PP, U>,
        baozi_operation::__canonical_activation::CanonicalJointSeal<'joint>,
    ) -> R,
) -> Result<R, baozi_operation::__canonical_activation::StoreActivationContainment>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    consume_node_scene_freeze_permit(permit, parser_draft.map(|draft| draft.0), reject, prepare)
}

/// Returns the exact protected terminal-work requirement shared by every NodeScene request.
pub fn scene_failure_terminal_work_requirement()
-> Result<RetainedStoreFailureTerminalWorkRequirement, baozi_operation::StorePlanError> {
    failure_terminal_work_requirement()
}

/// Returns the exact maximum number of retained members in rich-failure evidence.
pub const fn scene_failure_evidence_member_count() -> usize {
    FAILURE_TERMINAL_EVIDENCE_MEMBERS
}

/// Returns the exact per-pass field work for rich-failure construction and validation.
pub const fn scene_failure_evidence_field_work() -> u64 {
    NODE_SCENE_EVIDENCE_FIELD_COUNT as u64
}

/// Authority-free request construction failure.
#[derive(Debug)]
pub struct SceneActivationRequestError(SceneConstructionStartError);

impl fmt::Display for SceneActivationRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for SceneActivationRequestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestPreparedSceneSuccess = PreparedStoreSuccess<
        RecordBlock,
        SceneCanonicalRootTag,
        RecordBlock,
        SceneOutcomeEvidenceTag,
    >;

    #[test]
    fn lane_only_bridge_exposes_authority_free_core_steps() {
        fn _split_request<'input>(
            request: SceneActivationRequest<'input>,
        ) -> (
            SceneActivationRequirements,
            SceneActivationTransaction<'input>,
        ) {
            request.into_lane_parts()
        }

        fn _finish_transaction<'input>(
            transaction: SceneActivationTransaction<'input>,
        ) -> SceneActivationPostflight {
            transaction.finish()
        }
        let _: fn(&SceneActivationRequirements) -> baozi_operation::RetainedStoreBatchLimits =
            SceneActivationRequirements::canonical_writer_limits;
        let _: fn(&SceneActivationRequirements) -> baozi_operation::RetainedStoreBatchLimits =
            SceneActivationRequirements::outcome_evidence_writer_limits;
        let _: fn(SceneActivationRequirements) -> CanonicalFailureRequirements =
            SceneActivationRequirements::into_failure_requirements;
        let _: fn(&SceneActivationOutput) -> SceneDraft = SceneActivationOutput::draft;
        let _: fn(&SceneActivationRejection) -> u32 = SceneActivationRejection::stable_code;
        let _: fn(&SceneGenerationRejection) -> u32 = SceneGenerationRejection::stable_code;
        let _: fn(
            SceneHandoffToken,
            TestPreparedSceneSuccess,
        ) -> Result<ScenePublishedSuccess, SceneHandoffPairingError> = SceneHandoffToken::handoff;

        fn _materializes_only_from_lane<'input, 'scope, 'lane, O, PP, U, L>(
            plan: &SceneSealedActivationPlan<'input, 'scope, O>,
            lane: baozi_operation::__canonical_activation::CanonicalLane<'lane, O, PP, U, L>,
        ) -> baozi_operation::__canonical_activation::CanonicalMaterialization<
            'lane,
            SceneActivationOutput,
            SceneActivationRejection,
            RecordBlock,
            SceneCanonicalRootTag,
        >
        where
            O: OperationOwner,
            PP: ProgressPhase,
            U: ProgressUnit,
        {
            let _: CanonicalSuccessRequirements = plan.success_requirements();
            plan.materialize(lane)
        }
    }
}
