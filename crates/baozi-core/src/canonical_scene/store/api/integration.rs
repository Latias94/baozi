//! Hidden cross-crate integration boundary for canonical Scene transactions.
//!
//! A restored finite Core Candidate issues one freeze decision and hands its Ready permit
//! to the concrete consumer. Both non-Ready branches preserve the existing minimal policy:
//!
//! ```
//! use baozi_core::{ValidatedScene, __scene_integration::integration::{
//!     FiniteSceneEditFreezePermit, FiniteSceneEditPublishedSuccess, PreparedFiniteSceneEditPublication,
//!     PreparedFiniteSceneEditSuccess, SceneCanonicalRootTag, SceneEditEvidence,
//!     SceneEditEvidenceTag, SceneEditGenerationRejection, SceneEditOutput,
//!     SceneEditValidationProof, consume_finite_scene_edit_freeze_permit,
//! }};
//! use baozi_operation::__retained_store_integration::{
//!     activation::{CanonicalFinal, CanonicalGenerationSlots, FiniteCentralCandidate,
//!         FiniteCanonicalFreezePermitIssue},
//!     authority::FiniteStoreExitError,
//! };
//! use baozi_operation::{FinalProgress, FiniteStoreFacts, OperationOwner, ProgressPhase,
//!     ProgressUnit, RecordBlock};
//!
//! type Candidate<'inv, O, P, U> = CanonicalFinal<FiniteCentralCandidate<
//!     'inv, O, P, U, SceneEditOutput, SceneEditValidationProof,
//!     RecordBlock, SceneCanonicalRootTag, RecordBlock, SceneEditEvidenceTag,
//! >>;
//!
//! fn prepare<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
//!     candidate: Candidate<'inv, O, P, U>,
//!     generation: CanonicalGenerationSlots<'inv, SceneEditGenerationRejection>,
//!     progress: FinalProgress<P, U>,
//! ) -> Result<PreparedFiniteSceneEditPublication, FiniteStoreExitError<O, P, U>> {
//!     match candidate.try_issue_freeze_permit(generation.interruption, progress)? {
//!         FiniteCanonicalFreezePermitIssue::Ready(permit) => {
//!             let permit: FiniteSceneEditFreezePermit<'inv, O, P, U> = permit;
//!             consume_finite_scene_edit_freeze_permit(permit, generation.rejection)
//!         }
//!         FiniteCanonicalFreezePermitIssue::Interrupted(failure) => {
//!             let (store, _) = failure.prepare_minimal_fallback()?.into_parts();
//!             Ok(PreparedFiniteSceneEditPublication::MinimalFallback { store })
//!         }
//!         FiniteCanonicalFreezePermitIssue::DestinationRejected(failure) => {
//!             let (store, _) = failure.prepare_minimal_fallback()?.into_parts();
//!             Ok(PreparedFiniteSceneEditPublication::MinimalFallback { store })
//!         }
//!     }
//! }
//!
//! fn handoff(prepared: PreparedFiniteSceneEditSuccess)
//!     -> (ValidatedScene, SceneEditEvidence, FiniteStoreFacts)
//! {
//!     let published: FiniteSceneEditPublishedSuccess = prepared.handoff();
//!     published.into_parts()
//! }
//! ```
//!
//! The body Candidate retains its original unprepared capability:
//!
//! ```compile_fail,E0599
//! use baozi_core::__scene_integration::integration::{SceneCanonicalRootTag,
//!     SceneEditEvidenceTag, SceneEditOutput, SceneEditValidationProof};
//! use baozi_operation::__retained_store_integration::activation::{CanonicalCauseSlot,
//!     CanonicalFreezeInterruption, FiniteCentralCandidate};
//! use baozi_operation::{FinalProgress, OperationOwner, ProgressPhase, ProgressUnit, RecordBlock};
//! fn freeze_early<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
//!     candidate: FiniteCentralCandidate<'inv, O, P, U, SceneEditOutput,
//!         SceneEditValidationProof, RecordBlock, SceneCanonicalRootTag,
//!         RecordBlock, SceneEditEvidenceTag>,
//!     slot: CanonicalCauseSlot<'inv, CanonicalFreezeInterruption>,
//!     progress: FinalProgress<P, U>,
//! ) {
//!     let _ = candidate.try_issue_freeze_permit(slot, progress);
//! }
//! ```
//!
//! The dynamic permit cannot substitute for the finite permit:
//!
//! ```compile_fail,E0308
//! use baozi_core::__scene_integration::integration::{SceneEditFreezePermit,
//!     SceneEditGenerationRejection, consume_finite_scene_edit_freeze_permit};
//! use baozi_operation::__retained_store_integration::activation::{CanonicalCauseSlot,
//!     StoreCanonicalRejection};
//! use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
//! fn substitute<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
//!     permit: SceneEditFreezePermit<'inv, O, P, U>,
//!     slot: CanonicalCauseSlot<'inv, StoreCanonicalRejection<SceneEditGenerationRejection>>,
//! ) {
//!     let _ = consume_finite_scene_edit_freeze_permit(permit, slot);
//! }
//! ```
//!
//! Each nested finite/dynamic joint must consume its own invariant seal:
//!
//! ```compile_fail
//! use baozi_core::__scene_integration::integration::{FiniteSceneEditFreezePermit,
//!     SceneEditFreezePermit};
//! use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
//! fn mix<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
//!     finite: FiniteSceneEditFreezePermit<'inv, O, P, U>,
//!     dynamic: SceneEditFreezePermit<'inv, O, P, U>,
//! ) {
//!     finite.bind_joint_commit(|finite_joint, finite_seal| {
//!         dynamic.bind_joint_commit(|dynamic_joint, dynamic_seal| {
//!             let _ = finite_joint.prepare(dynamic_seal);
//!             let _ = dynamic_joint.prepare(finite_seal);
//!         })
//!     })
//! }
//! ```
//!
//! Prepared finite success keeps the matching token and physical store private:
//!
//! ```compile_fail,E0616
//! use baozi_core::__scene_integration::integration::PreparedFiniteSceneEditSuccess;
//! fn extract_token(prepared: PreparedFiniteSceneEditSuccess) { let _ = prepared.token; }
//! fn extract_store(prepared: PreparedFiniteSceneEditSuccess) { let _ = prepared.store; }
//! ```
//!
//! Its handoff consumes the sole prepared owner:
//!
//! ```compile_fail,E0382
//! use baozi_core::__scene_integration::integration::PreparedFiniteSceneEditSuccess;
//! fn handoff_twice(prepared: PreparedFiniteSceneEditSuccess) {
//!     let _ = prepared.handoff();
//!     let _ = prepared.handoff();
//! }
//! ```
//!
//! A finite Ready permit cannot outlive its original Host loan:
//!
//! ```compile_fail
//! use baozi_core::__scene_integration::integration::FiniteSceneEditFreezePermit;
//! use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
//! fn escape<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
//!     permit: FiniteSceneEditFreezePermit<'inv, O, P, U>,
//! ) -> FiniteSceneEditFreezePermit<'static, O, P, U> {
//!     permit
//! }
//! ```
//!
//! The bound finite entry preserves both its original Host and source loans:
//!
//! ```
//! use baozi_core::{ValidatedScene,
//!     __scene_integration::integration::run_finite_scene_edit_body};
//! use baozi_operation::__retained_store_integration::{
//!     activation::{FiniteCanonicalBoundInvocationFinish, FiniteCanonicalInvocationHost},
//!     authority::{FiniteRetainedStoreStartPlan, FiniteStoreExitError, FiniteStoreLedgerPlan},
//! };
//! use baozi_operation::{OperationControl, OperationOwner, ProgressPhase, ProgressUnit};
//! type Finish<'host, 'source, O, P, U> = FiniteCanonicalBoundInvocationFinish<
//!     'host, 'source, O, P, U, (), (), (), (), (), (), (), (), (),
//! >;
//! fn bind<'host, 'source, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
//!     host: &'host mut FiniteCanonicalInvocationHost<O, P, U, (), ()>,
//!     scene: &'source ValidatedScene,
//!     control: OperationControl<P, U>,
//!     ledger: FiniteStoreLedgerPlan,
//!     plan: FiniteRetainedStoreStartPlan,
//! ) -> Result<Finish<'host, 'source, O, P, U>, FiniteStoreExitError<O, P, U>> {
//!     run_finite_scene_edit_body(host, scene, control, ledger, plan, |parts, disposition| {
//!         parts.startup.request_unchanged(disposition).unwrap();
//!         Ok(())
//!     })
//! }
//! ```
//!
//! That same entry cannot turn the source borrow into a static finish:
//!
//! ```compile_fail
//! use baozi_core::{ValidatedScene,
//!     __scene_integration::integration::run_finite_scene_edit_body};
//! use baozi_operation::__retained_store_integration::{
//!     activation::{FiniteCanonicalBoundInvocationFinish, FiniteCanonicalInvocationHost},
//!     authority::{FiniteRetainedStoreStartPlan, FiniteStoreExitError, FiniteStoreLedgerPlan},
//! };
//! use baozi_operation::{OperationControl, OperationOwner, ProgressPhase, ProgressUnit};
//! type Finish<'host, 'source, O, P, U> = FiniteCanonicalBoundInvocationFinish<
//!     'host, 'source, O, P, U, (), (), (), (), (), (), (), (), (),
//! >;
//! fn escape<'host, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
//!     host: &'host mut FiniteCanonicalInvocationHost<O, P, U, (), ()>,
//!     scene: &ValidatedScene,
//!     control: OperationControl<P, U>,
//!     ledger: FiniteStoreLedgerPlan,
//!     plan: FiniteRetainedStoreStartPlan,
//! ) -> Result<Finish<'host, 'static, O, P, U>, FiniteStoreExitError<O, P, U>> {
//!     run_finite_scene_edit_body(host, scene, control, ledger, plan, |parts, disposition| {
//!         parts.startup.request_unchanged(disposition).unwrap();
//!         Ok(())
//!     })
//! }
//! ```

use baozi_operation::__retained_store_integration::activation::{
    CanonicalCauseSlot, StoreCanonicalRejection,
};
use baozi_operation::{
    OperationOwner, ProgressPhase, ProgressUnit, PublishedOutcomeEvidenceStore, RecordBlock,
    RetainedStoreFailureTerminalWorkRequirement, StoreLedgerEvidence,
};

pub use super::super::construction::node_scene::edit::SceneEditCompileError;
pub use super::super::construction::node_scene::edit::transaction::{
    FiniteSceneEditFreezePermit, FiniteSceneEditPublishedSuccess,
    PreparedFiniteSceneEditPublication, PreparedFiniteSceneEditSuccess,
    ResolvedSceneSpaceConversion, SceneEditAdmission, SceneEditBootstrap, SceneEditBootstrapError,
    SceneEditEvidence, SceneEditEvidenceTag, SceneEditFailureEvidence, SceneEditFreezePermit,
    SceneEditGenerationRejection, SceneEditOutput, SceneEditPlan, SceneEditPostflight,
    SceneEditPublication, SceneEditPublicationError, SceneEditPublishedSuccess, SceneEditRejection,
    SceneEditTarget, SceneEditTransaction, SceneEditValidationProof, SceneGeometryCornerOriginsTag,
    SceneGeometryFaceOffsetsTag, SceneSpaceConversionEvidence, SceneSpaceConversionIssue,
    consume_finite_scene_edit_freeze_permit, consume_scene_edit_freeze_permit,
    resolve_scene_space_conversion, run_finite_scene_edit_body, run_scene_edit_body,
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
        let _: fn(PreparedFiniteSceneEditSuccess) -> FiniteSceneEditPublishedSuccess =
            PreparedFiniteSceneEditSuccess::handoff;
        let _: fn(
            FiniteSceneEditPublishedSuccess,
        ) -> (
            ValidatedScene,
            SceneEditEvidence,
            baozi_operation::FiniteStoreFacts,
        ) = FiniteSceneEditPublishedSuccess::into_parts;
    }
}
