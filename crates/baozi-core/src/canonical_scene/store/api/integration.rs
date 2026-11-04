//! Hidden cross-crate integration boundary for canonical Scene transactions.

use baozi_operation::__retained_store_integration::activation::{
    CanonicalCauseSlot, StoreCanonicalRejection,
};
use baozi_operation::{
    OperationOwner, ProgressPhase, ProgressUnit, PublishedOutcomeEvidenceStore, RecordBlock,
    RetainedStoreFailureTerminalWorkRequirement, StoreLedgerEvidence,
};

pub use super::super::construction::node_scene::edit::SceneEditCompileError;
pub use super::super::construction::node_scene::edit::transaction::{
    ResolvedSceneSpaceConversion, SceneEditAdmission, SceneEditBootstrap, SceneEditBootstrapError,
    SceneEditEvidence, SceneEditEvidenceTag, SceneEditFailureEvidence, SceneEditFreezePermit,
    SceneEditGenerationRejection, SceneEditOutput, SceneEditPlan, SceneEditPostflight,
    SceneEditPublication, SceneEditPublicationError, SceneEditPublishedSuccess, SceneEditRejection,
    SceneEditTarget, SceneEditTransaction, SceneEditValidationProof, SceneGeometryCornerOriginsTag,
    SceneGeometryFaceOffsetsTag, SceneSpaceConversionEvidence, SceneSpaceConversionIssue,
    consume_scene_edit_freeze_permit, resolve_scene_space_conversion, run_scene_edit_body,
    try_start_scene_edit_operation,
};
pub use super::super::construction::node_scene::edit::transaction::{
    SceneGeometryPlanScratch, SceneGeometryPlanStorage, SceneGeometryReplacement,
    SceneGeometrySelectionEvidence, SceneGeometryTargetEvidence, SceneTriangleOrigins,
};
pub use super::super::construction::node_scene::{
    NodeSceneActivationPostflight as SceneActivationPostflight,
    NodeSceneActivationTransaction as SceneActivationTransaction,
    NodeSceneEvidenceRecipe as SceneEvidenceRecipe, NodeSceneLaneFreezePermit as SceneFreezePermit,
    NodeSceneLaneGenerationRejection as SceneGenerationRejection,
    NodeSceneLaneOutput as SceneActivationOutput, NodeSceneLanePlan as SceneSealedActivationPlan,
    NodeSceneLaneRejection as SceneActivationRejection,
    NodeSceneLaneValidationProof as SceneValidationProof,
    NodeScenePublicationError as ScenePublicationError,
    NodeSceneSessionActivationBootstrap as SceneSessionActivationBootstrap,
    PreparedNodeScenePublication as PreparedScenePublication,
    PreparedNodeSceneSuccess as PreparedSceneSuccess, SceneParseOutcome,
};
use super::super::construction::node_scene::{
    consume_node_scene_freeze_permit, maximum_node_scene_structural_work,
};
use super::super::construction::{
    FAILURE_TERMINAL_EVIDENCE_MEMBERS, failure_terminal_work_requirement,
};
use super::super::schema::{NODE_SCENE_EVIDENCE_FIELD_COUNT, SCENE_DIRECTORY_BLOCK_COUNT};
pub use super::super::schema::{
    NodeSceneOutcomeEvidenceSchema as SceneOutcomeEvidenceTag,
    SceneDirectorySchema as SceneCanonicalRootTag,
};
pub use super::super::typed_storage::edit::VectorValues as SceneVectorValues;
pub use super::construction::{SceneConstruction, SceneConstructionStartError};
use super::scene::ValidatedScene;

/// Returns the maximum number of remaining record-store blocks in a fresh Scene.
pub const fn maximum_scene_canonical_blocks() -> usize {
    SCENE_DIRECTORY_BLOCK_COUNT as usize
}

/// Returns the protected structural-work ceiling for the remaining record-store domains.
pub fn maximum_scene_structural_work() -> Option<u64> {
    let canonical_blocks = maximum_scene_canonical_blocks();
    maximum_node_scene_structural_work(canonical_blocks)
}

impl<'input> SceneConstruction<'input> {
    /// Moves the authority-free construction description into its semantic transaction.
    pub fn into_transaction(self) -> SceneActivationTransaction<'input> {
        self.into_node_scene_request().into_transaction()
    }
}

impl PreparedSceneSuccess {
    /// Transfers the success owners whose exact roots were admitted before generation.
    pub fn handoff(self) -> ScenePublishedSuccess {
        ScenePublishedSuccess(self.handoff_node_scene())
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

/// Consumes the freeze permit using the host's final parser outcome and core's actual candidate.
///
/// Rejection downgrades the candidate without reserving a generation. Acceptance admits a
/// generation for the validated output and proof retained by this permit.
/// Core prepares the matching operation contribution and returns fixed, inseparable owners.
#[allow(
    clippy::result_large_err,
    clippy::type_complexity,
    reason = "the freeze pivot preserves exact containment and scoped joint ownership without boxing"
)]
pub fn consume_scene_freeze_permit<'inv, O, PP, U>(
    permit: SceneFreezePermit<'inv, O, PP, U>,
    rejection_slot: CanonicalCauseSlot<'inv, StoreCanonicalRejection<SceneGenerationRejection>>,
    failure_fallback: CanonicalCauseSlot<'inv, baozi_operation::StoreRuntimeError>,
    evidence: SceneEvidenceRecipe,
    parse_outcome: SceneParseOutcome,
) -> Result<PreparedScenePublication, ScenePublicationError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    consume_node_scene_freeze_permit(
        permit,
        rejection_slot,
        failure_fallback,
        evidence,
        parse_outcome,
    )
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

#[cfg(test)]
pub use super::super::construction::node_scene::edit::transaction::{
    SceneNormalGenerationEvidence, SceneNormalReplacement, SceneTangentGenerationEvidence,
    SceneTangentReplacement, SceneTriangulationEvidence, SceneTriangulationReplacement,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_integration_exposes_authority_free_core_steps() {
        fn _split_construction<'input>(
            construction: SceneConstruction<'input>,
        ) -> SceneActivationTransaction<'input> {
            construction.into_transaction()
        }

        fn _finish_transaction<'input>(
            transaction: SceneActivationTransaction<'input>,
        ) -> SceneActivationPostflight {
            transaction.finish()
        }
        let _: fn() -> Result<SceneSessionActivationBootstrap, baozi_operation::StorePlanError> =
            SceneSessionActivationBootstrap::try_new;
        let _: fn(&SceneActivationRejection) -> u32 = SceneActivationRejection::stable_code;
        let _: fn(&SceneGenerationRejection) -> u32 = SceneGenerationRejection::stable_code;
        let _: fn(PreparedSceneSuccess) -> ScenePublishedSuccess = PreparedSceneSuccess::handoff;
    }
}
