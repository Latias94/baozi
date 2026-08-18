//! Hidden cross-crate bridge for the U4 canonical import cutover.

use std::fmt;
use std::num::NonZeroU64;

use baozi_operation::__canonical_activation::{
    CanonicalFailureRequirements, CanonicalSuccessRequirements, CanonicalTableShape,
    CanonicalValidationProgress,
};
use baozi_operation::{
    ControlCause, FinalProgress, OperationOwner, ProgressPhase, ProgressUnit,
    RetainedStoreFailureTerminalWorkRequirement, RetainedStoreOperation, StoreDimensionKey,
    StoreLedgerEvidence, StorePhaseKey,
};

use crate::identity::SceneGenerationId;
use crate::validation_control::SceneValidationLimits;

use super::super::construction::node_scene::{
    DeferredNodeSceneDraftBuild, NodeSceneActivationRequest, NodeSceneActivationRoles,
    NodeSceneConstructionTerminal, NodeSceneRichFailure, NodeSceneSuccessParts,
    NodeSceneSuccessReport, NodeSceneWriterRoles, PendingNodeSceneDraft, PendingNodeSceneFailure,
    PreparedNodeSceneConstruction, prepare_unactivated_node_scene,
};
use super::super::construction::{
    FAILURE_TERMINAL_EVIDENCE_MEMBERS, SceneConstructionError, SceneConstructionStartError,
    TracerSceneFallback, failure_terminal_work_requirement,
};
use super::super::production::SceneInputEnvelope;
use super::super::schema::NODE_SCENE_EVIDENCE_FIELD_COUNT;
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
    pub fn try_new(
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
    ) -> Result<Self, SceneActivationRequestError> {
        NodeSceneActivationRequest::try_new(input, validation_limits)
            .map(Self)
            .map_err(SceneActivationRequestError)
    }

    /// Consumes the whole request and binds it to the host's sole operation and role mapping.
    pub fn activate<O, PP, U>(
        self,
        operation: RetainedStoreOperation<O, PP, U>,
        roles: SceneActivationRoles,
        validation_progress: CanonicalValidationProgress<PP, U>,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<SceneDraftBuild<O, PP, U>, SceneActivationError>
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        self.0
            .activate_deferred(operation, roles.0, validation_progress, final_progress)
            .map(SceneDraftBuild::from_node_scene)
            .map_err(SceneActivationError)
    }
}

/// One writer's host-owned phase, capacity, and attribution roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneWriterRoles(NodeSceneWriterRoles);

impl SceneWriterRoles {
    /// Binds one roleless writer envelope to host-owned store keys.
    pub const fn new(
        phase: StorePhaseKey,
        capacity_dimension: StoreDimensionKey,
        attribution_dimension: StoreDimensionKey,
    ) -> Self {
        Self(NodeSceneWriterRoles::new(
            phase,
            capacity_dimension,
            attribution_dimension,
        ))
    }
}

/// Complete host-owned role mapping for one canonical Scene activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneActivationRoles(NodeSceneActivationRoles);

impl SceneActivationRoles {
    /// Creates a complete key mapping without accepting request-derived limits or execution data.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        canonical_writer: SceneWriterRoles,
        outcome_evidence_writer: SceneWriterRoles,
        materialization_work_dimension: StoreDimensionKey,
        materialization_scratch_dimension: StoreDimensionKey,
        validation_phase: StorePhaseKey,
        validation_work_dimension: StoreDimensionKey,
        validation_scratch_dimension: StoreDimensionKey,
        canonical_destination_dimension: StoreDimensionKey,
        outcome_evidence_destination_dimension: StoreDimensionKey,
    ) -> Self {
        Self(NodeSceneActivationRoles::new(
            canonical_writer.0,
            outcome_evidence_writer.0,
            materialization_work_dimension,
            materialization_scratch_dimension,
            validation_phase,
            validation_work_dimension,
            validation_scratch_dimension,
            canonical_destination_dimension,
            outcome_evidence_destination_dimension,
        ))
    }

    /// Projects exact retained Scene string bytes into the import host's canonical string role.
    pub const fn with_canonical_string_dimension(mut self, dimension: StoreDimensionKey) -> Self {
        self.0 = self.0.with_canonical_string_dimension(dimension);
        self
    }
}

/// Structural owner for the retained blocks referenced by one [`SceneDraft`].
///
/// Preparation consumes both values. A missing or foreign descriptor becomes an ordinary frozen
/// failure before generation reservation.
#[must_use = "the pending Scene draft must be prepared or deliberately dropped"]
pub struct PendingSceneDraftOwner<O, PP, U>(PendingNodeSceneDraft<O, PP, U>)
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit;

impl<O, PP, U> PendingSceneDraftOwner<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Verifies the parser result, reserves one generation only for an exact match, and freezes
    /// the terminal publication.
    pub fn prepare(
        self,
        draft: Option<SceneDraft>,
    ) -> Result<PreparedSceneConstruction, SceneActivationError> {
        let (prepared, control_cause) = match draft {
            Some(draft) => self
                .0
                .prepare_import_with_generation_source(draft.0, SceneGenerationId::reserve),
            None => self.0.prepare_import_missing_draft(),
        }
        .map_err(SceneActivationError)?;
        Ok(PreparedSceneConstruction(prepared, control_cause))
    }
}

/// Linear failure continuation retained until parser-owned storage has been released.
#[must_use = "the pending Scene failure must be prepared or deliberately dropped"]
pub struct PendingSceneFailureOwner<O, PP, U>(PendingNodeSceneFailure<O, PP, U>)
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit;

impl<O, PP, U> PendingSceneFailureOwner<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Builds, validates, and freezes the terminal failure after parser storage is quiescent.
    pub fn prepare(self) -> Result<PreparedSceneConstruction, SceneActivationError> {
        let (prepared, _authoring_rejection, control_cause) =
            self.0.prepare().map_err(SceneActivationError)?;
        Ok(PreparedSceneConstruction(prepared, control_cause))
    }
}

/// Opaque activation result which separates the parser descriptor from its retained owner.
#[must_use = "the Scene draft build must be installed in its import session"]
#[allow(
    clippy::large_enum_variant,
    reason = "the failure owner stays inline so fail-closed transitions do not allocate"
)]
pub enum SceneDraftBuild<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Materialization succeeded and awaits the parser's exact descriptor.
    Draft {
        draft: SceneDraft,
        owner: PendingSceneDraftOwner<O, PP, U>,
    },
    /// Activation retained an authoritative failure continuation for later preparation.
    Failure {
        authoring_rejected: bool,
        draft_built: bool,
        control_cause: Option<ControlCause>,
        // Keep the physical continuation last so future classification fields cannot unwind after
        // its containment owner has already been released.
        owner: PendingSceneFailureOwner<O, PP, U>,
    },
}

impl<O, PP, U> SceneDraftBuild<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn from_node_scene(build: DeferredNodeSceneDraftBuild<O, PP, U>) -> Self {
        match build {
            DeferredNodeSceneDraftBuild::Draft { draft, owner } => Self::Draft {
                draft: SceneDraft(draft),
                owner: PendingSceneDraftOwner(owner),
            },
            DeferredNodeSceneDraftBuild::Failure {
                authoring_rejected,
                draft_built,
                control_cause,
                owner,
            } => Self::Failure {
                authoring_rejected,
                draft_built,
                control_cause,
                owner: PendingSceneFailureOwner(owner),
            },
        }
    }
}

/// Frozen publication whose remaining handoff is a pure ownership move.
#[must_use = "the prepared Scene construction must be handed off or deliberately dropped"]
pub struct PreparedSceneConstruction(PreparedNodeSceneConstruction, Option<ControlCause>);

impl PreparedSceneConstruction {
    /// Moves the frozen publication into its final canonical Scene or failure owner.
    pub fn handoff(self) -> SceneConstructionTerminal {
        SceneConstructionTerminal(self.0.handoff(), self.1)
    }
}

/// Freezes the pre-admitted fallback when an import ends before submitting a Scene request.
///
/// This consumes the same retained operation that accounted producer work. It creates no empty
/// Scene, generation, canonical commitment, or second ledger.
pub fn prepare_unactivated_scene<O, PP, U>(
    operation: RetainedStoreOperation<O, PP, U>,
    stable_code: u32,
) -> Result<PreparedSceneConstruction, SceneActivationError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    prepare_unactivated_node_scene(operation, stable_code)
        .map(|prepared| PreparedSceneConstruction(prepared, None))
        .map_err(SceneActivationError)
}

/// Final canonical Scene construction outcome retained by the import session.
#[must_use = "the terminal owns the exact canonical Scene construction outcome"]
pub struct SceneConstructionTerminal(NodeSceneConstructionTerminal, Option<ControlCause>);

impl SceneConstructionTerminal {
    /// Returns whether canonical Scene construction and joint publication succeeded.
    pub const fn is_success(&self) -> bool {
        matches!(self.0, NodeSceneConstructionTerminal::Success(_))
    }

    /// Returns the retained canonical member count for a successful Scene.
    pub fn canonical_block_count(&self) -> Option<usize> {
        match &self.0 {
            NodeSceneConstructionTerminal::Success(NodeSceneSuccessParts { scene, .. }) => {
                Some(scene.canonical_block_count())
            }
            NodeSceneConstructionTerminal::RichFailure(_)
            | NodeSceneConstructionTerminal::MinimalFallback(_) => None,
        }
    }

    /// Borrows the unique frozen ledger evidence for this terminal outcome.
    pub const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        match &self.0 {
            NodeSceneConstructionTerminal::Success(parts) => parts.report.ledger_evidence(),
            NodeSceneConstructionTerminal::RichFailure(failure) => failure.ledger_evidence(),
            NodeSceneConstructionTerminal::MinimalFallback(fallback) => fallback.ledger_evidence(),
        }
    }

    /// Returns the exact operation-control cause retained alongside a Control terminal.
    pub const fn control_cause(&self) -> Option<&ControlCause> {
        self.1.as_ref()
    }

    /// Consumes the terminal into the exact retained owners published by canonical construction.
    ///
    /// This is an ownership projection only: it does not copy ledger evidence, inspect raw
    /// canonical storage, or perform another publication step.
    pub fn into_parts(self) -> SceneConstructionTerminalParts {
        match self.0 {
            NodeSceneConstructionTerminal::Success(NodeSceneSuccessParts { scene, report }) => {
                SceneConstructionTerminalParts::Success {
                    scene: ValidatedScene::from_node_scene(scene),
                    report: SceneSuccessReport(report),
                }
            }
            NodeSceneConstructionTerminal::RichFailure(failure) => {
                SceneConstructionTerminalParts::RichFailure(SceneRichFailure(failure))
            }
            NodeSceneConstructionTerminal::MinimalFallback(fallback) => {
                SceneConstructionTerminalParts::MinimalFallback(SceneMinimalFallback(fallback))
            }
        }
    }
}

/// Consuming projection of the exact owners retained by one terminal publication.
#[must_use = "the terminal parts own the published canonical Scene or failure evidence"]
#[allow(
    clippy::large_enum_variant,
    reason = "published owners remain inline so the projection is a pure ownership move"
)]
pub enum SceneConstructionTerminalParts {
    /// Independently owned canonical Scene and successful outcome evidence.
    Success {
        /// Centrally validated canonical Scene owner.
        scene: ValidatedScene,
        /// Outcome evidence published in the same joint commit as the canonical Scene.
        report: SceneSuccessReport,
    },
    /// Structured failure evidence published without canonical Scene ownership.
    RichFailure(SceneRichFailure),
    /// Bounded fail-closed owner used when rich evidence could not be published.
    MinimalFallback(SceneMinimalFallback),
}

/// Opaque owner of outcome evidence jointly published with a successful Scene.
#[must_use = "the success report owns published outcome evidence"]
pub struct SceneSuccessReport(NodeSceneSuccessReport);

impl SceneSuccessReport {
    /// Returns the number of retained outcome-evidence blocks.
    pub fn outcome_evidence_block_count(&self) -> usize {
        self.0.outcome_evidence_block_count()
    }

    /// Borrows the unique frozen ledger evidence for this successful outcome.
    pub const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        self.0.ledger_evidence()
    }
}

/// Opaque owner of one structured canonical-construction failure publication.
#[must_use = "the rich failure owns published outcome evidence"]
pub struct SceneRichFailure(NodeSceneRichFailure);

impl SceneRichFailure {
    /// Returns the number of retained outcome-evidence blocks.
    pub fn outcome_evidence_block_count(&self) -> usize {
        self.0.outcome_evidence_block_count()
    }

    /// Borrows the unique frozen ledger evidence for this failed outcome.
    pub const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        self.0.ledger_evidence()
    }
}

/// Opaque owner of one bounded fail-closed canonical-construction publication.
#[must_use = "the fallback owns its bounded published failure state"]
pub struct SceneMinimalFallback(TracerSceneFallback);

impl SceneMinimalFallback {
    /// Returns the stable failure code retained by the bounded fallback.
    pub fn stable_code(&self) -> u32 {
        self.0.fallback_code()
    }

    /// Borrows the unique frozen ledger evidence for this fail-closed outcome.
    pub const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        self.0.ledger_evidence()
    }
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

/// Fail-closed activation or terminal preparation error.
#[derive(Debug)]
pub struct SceneActivationError(SceneConstructionError);

impl fmt::Display for SceneActivationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for SceneActivationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_projection_requires_ownership_and_exposes_only_safe_views() {
        fn accepts_internal_scene(_: &super::super::ValidatedScene) {}

        let _: fn(&ValidatedScene) = accepts_internal_scene;
        let _: fn(SceneConstructionTerminal) -> SceneConstructionTerminalParts =
            SceneConstructionTerminal::into_parts;
        let _: fn(&ValidatedScene) -> SceneGenerationId = ValidatedScene::generation;
        let _: fn(&ValidatedScene) -> usize = ValidatedScene::canonical_block_count;
        let _: fn(&SceneSuccessReport) -> usize = SceneSuccessReport::outcome_evidence_block_count;
        let _: for<'a> fn(&'a SceneSuccessReport) -> &'a StoreLedgerEvidence =
            SceneSuccessReport::ledger_evidence;
        let _: fn(&SceneRichFailure) -> usize = SceneRichFailure::outcome_evidence_block_count;
        let _: for<'a> fn(&'a SceneRichFailure) -> &'a StoreLedgerEvidence =
            SceneRichFailure::ledger_evidence;
        let _: fn(&SceneMinimalFallback) -> u32 = SceneMinimalFallback::stable_code;
        let _: for<'a> fn(&'a SceneMinimalFallback) -> &'a StoreLedgerEvidence =
            SceneMinimalFallback::ledger_evidence;
    }
}
