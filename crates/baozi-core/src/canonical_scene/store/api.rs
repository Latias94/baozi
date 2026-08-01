#![allow(
    clippy::result_large_err,
    reason = "the opaque lifecycle preserves retained construction errors inline so fail-closed paths do not allocate"
)]

//! Opaque final-name API over the retained canonical Scene construction rail.
//!
//! A parser-facing [`SceneDraft`] is deliberately separated from its
//! [`PendingSceneDraftOwner`]. The descriptor carries no retained ownership, while the owner
//! structurally keeps every materialized block alive until central preparation consumes both.

use baozi_operation::{PhysicalStorageAccount, StoreLedgerEvidence};

use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};

pub(crate) use super::construction::SceneConstructionError;
use super::construction::TracerSceneFallback;
use super::construction::node_scene::{
    NodeSceneConstructionTerminal, NodeSceneDraft, NodeSceneDraftBuild, NodeSceneRichFailure,
    NodeSceneSuccessParts, NodeSceneSuccessReport, PendingNodeSceneDraft,
    PreparedNodeSceneConstruction, PreparedNodeSceneFailure, ValidatedNodeScene,
    ValidatedNodeSceneConstruction,
};

pub(crate) mod continuation;

/// A non-owning descriptor for one materialized canonical Scene directory.
///
/// This type is intentionally neither `Clone` nor `Copy`. The matching retained owner is carried
/// separately by [`PendingSceneDraftOwner`] and must be consumed with this descriptor.
#[must_use = "a Scene draft descriptor must be centrally prepared with its retained owner"]
pub(crate) struct SceneDraft(NodeSceneDraft);

/// Structural owner for all retained blocks referenced by one [`SceneDraft`].
///
/// Dropping this value releases the unpublished retained state. Preparation consumes both the
/// owner and descriptor so neither can survive a terminal publication decision.
#[must_use = "a pending Scene draft owner must be centrally prepared or deliberately dropped"]
struct PendingSceneDraftOwner(PendingNodeSceneDraft);

/// The exact result of materialization before central canonical validation.
#[must_use = "a Scene draft build owns either validation work or a prepared failure"]
#[allow(
    clippy::large_enum_variant,
    reason = "the failure owner stays inline so containment does not allocate"
)]
enum SceneDraftBuild {
    Draft {
        draft: SceneDraft,
        owner: PendingSceneDraftOwner,
    },
    Failure(PreparedSceneFailure),
}

impl SceneDraftBuild {
    fn from_node_scene(build: NodeSceneDraftBuild) -> Self {
        match build {
            NodeSceneDraftBuild::Draft { draft, owner } => Self::Draft {
                draft: SceneDraft(draft),
                owner: PendingSceneDraftOwner(owner),
            },
            NodeSceneDraftBuild::Failure(failure) => Self::Failure(PreparedSceneFailure(failure)),
        }
    }

    fn prepare_with_generation_source(
        self,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedSceneConstruction, SceneConstructionError> {
        match self {
            Self::Draft { draft, owner } => {
                owner.prepare_with_generation_source(draft, reserve_generation)
            }
            Self::Failure(failure) => Ok(failure.into()),
        }
    }
}

impl PendingSceneDraftOwner {
    fn prepare_with_generation_source(
        self,
        draft: SceneDraft,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedSceneConstruction, SceneConstructionError> {
        self.0
            .prepare_with_generation_source(draft.0, reserve_generation)
            .map(PreparedSceneConstruction)
    }

    fn prepare_missing_draft(self) -> Result<PreparedSceneConstruction, SceneConstructionError> {
        self.0
            .prepare_missing_draft()
            .map(PreparedSceneConstruction)
    }
}

/// A validated source batch coupled to the exact operation that admitted its work.
#[must_use = "the validated Scene construction must be materialized or deliberately dropped"]
struct ValidatedSceneConstruction<'input>(ValidatedNodeSceneConstruction<'input>);

impl<'input> ValidatedSceneConstruction<'input> {
    pub(in crate::canonical_scene::store) fn from_node_scene(
        construction: ValidatedNodeSceneConstruction<'input>,
    ) -> Self {
        Self(construction)
    }

    /// Materializes the source-backed input into a descriptor and its independent retained owner.
    fn finish_draft(self) -> Result<SceneDraftBuild, SceneConstructionError> {
        self.0.finish_draft().map(SceneDraftBuild::from_node_scene)
    }

    /// Exercises the final opaque handoff with a deterministic generation source.
    ///
    /// This is test-only so production callers can neither select nor observe the private
    /// generation reservation step.
    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn prepare_with_generation_source(
        self,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedSceneConstruction, SceneConstructionError> {
        self.finish_draft()?
            .prepare_with_generation_source(reserve_generation)
    }
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn handoff_validated_node_scene_with_generation_source_for_test(
    construction: ValidatedNodeSceneConstruction<'_>,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<SceneConstructionTerminal, SceneConstructionError> {
    ValidatedSceneConstruction::from_node_scene(construction)
        .prepare_with_generation_source(reserve_generation)
        .map(PreparedSceneConstruction::handoff)
}

/// A frozen failure publication which can only be forwarded to terminal handoff.
#[must_use = "a prepared Scene failure must be handed off or deliberately dropped"]
struct PreparedSceneFailure(PreparedNodeSceneFailure);

impl From<PreparedSceneFailure> for PreparedSceneConstruction {
    fn from(failure: PreparedSceneFailure) -> Self {
        Self(failure.0.into())
    }
}

/// A frozen terminal publication.
///
/// No content accessor is provided. [`Self::handoff`] only moves already-published ownership into
/// the final Scene/report/failure wrappers and cannot allocate new retained storage.
#[must_use = "a prepared Scene construction must be handed off or deliberately dropped"]
pub(crate) struct PreparedSceneConstruction(PreparedNodeSceneConstruction);

impl PreparedSceneConstruction {
    pub(crate) fn handoff(self) -> SceneConstructionTerminal {
        SceneConstructionTerminal::from_node_scene(self.0.handoff())
    }
}

/// The centrally validated canonical Scene owner.
#[must_use = "a validated Scene owns retained canonical storage"]
pub(crate) struct ValidatedScene(ValidatedNodeScene);

impl ValidatedScene {
    pub(crate) const fn generation(&self) -> SceneGenerationId {
        self.0.generation()
    }

    pub(crate) fn account(&self) -> &PhysicalStorageAccount {
        self.0.account()
    }

    pub(crate) fn canonical_block_count(&self) -> usize {
        self.0.canonical_block_count()
    }

    /// Restricts access to the sealed retained rail so later semantic views and SceneEdit can
    /// project domain data without exposing the raw canonical store.
    pub(in crate::canonical_scene::store) const fn as_node_scene(&self) -> &ValidatedNodeScene {
        &self.0
    }
}

/// Outcome evidence published alongside a successful Scene.
#[must_use = "the Scene success report owns published outcome evidence"]
pub(crate) struct SceneSuccessReport(NodeSceneSuccessReport);

impl SceneSuccessReport {
    pub(crate) fn outcome_evidence_block_count(&self) -> usize {
        self.0.outcome_evidence_block_count()
    }

    pub(crate) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        self.0.ledger_evidence()
    }
}

/// Independently owned canonical Scene and success evidence after handoff.
#[must_use = "the success parts own both the canonical Scene and its report"]
pub(crate) struct SceneSuccessParts {
    pub(crate) scene: ValidatedScene,
    pub(crate) report: SceneSuccessReport,
}

/// Structured evidence for a rich construction failure.
#[must_use = "the rich failure owns published outcome evidence"]
pub(crate) struct SceneRichFailure(NodeSceneRichFailure);

impl SceneRichFailure {
    pub(crate) fn account(&self) -> &PhysicalStorageAccount {
        self.0.account()
    }

    pub(crate) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        self.0.ledger_evidence()
    }

    pub(crate) fn outcome_evidence_block_count(&self) -> usize {
        self.0.outcome_evidence_block_count()
    }
}

/// The bounded fail-closed terminal when rich evidence cannot be published.
#[must_use = "the fallback owns its bounded published failure state"]
pub(crate) struct SceneFallback(TracerSceneFallback);

impl SceneFallback {
    pub(crate) fn account(&self) -> &PhysicalStorageAccount {
        self.0.account()
    }

    pub(crate) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        self.0.ledger_evidence()
    }
}

/// The exact terminal ownership outcome of canonical Scene construction.
#[must_use = "the terminal owns the exact canonical Scene construction outcome"]
#[allow(
    clippy::large_enum_variant,
    reason = "published owners remain inline so handoff is a pure ownership move"
)]
pub(crate) enum SceneConstructionTerminal {
    Success(SceneSuccessParts),
    RichFailure(SceneRichFailure),
    MinimalFallback(SceneFallback),
}

impl SceneConstructionTerminal {
    fn from_node_scene(terminal: NodeSceneConstructionTerminal) -> Self {
        match terminal {
            NodeSceneConstructionTerminal::Success(NodeSceneSuccessParts { scene, report }) => {
                Self::Success(SceneSuccessParts {
                    scene: ValidatedScene(scene),
                    report: SceneSuccessReport(report),
                })
            }
            NodeSceneConstructionTerminal::RichFailure(failure) => {
                Self::RichFailure(SceneRichFailure(failure))
            }
            NodeSceneConstructionTerminal::MinimalFallback(fallback) => {
                Self::MinimalFallback(SceneFallback(fallback))
            }
        }
    }
}
