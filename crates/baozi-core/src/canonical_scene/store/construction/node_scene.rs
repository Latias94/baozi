use baozi_operation::{
    CounterClass, LedgerCause, OutcomeEvidenceRecordRef, PhysicalStorageAccount,
    PreparedStoreFailureOutcome, PreparedStoreSuccessOutcome, PublishedCanonicalStore,
    PublishedOutcomeEvidenceStore, PublishedRichFailureStore, RecordBlock, RecordPlan,
    RetainedStoreBatchLimits, RetainedStoreBlockTablePlan, RetainedStorePublicationPlan,
    RetainedStoreReservation, RetainedStoreWorkControl, RetainedStoreWriterPlan, SequencePlan,
    StoreBuildError, StoreBuildFailureClass, StoreControlledError,
    StoreControlledSuccessValidation, StoreFailureAssembly, StoreFailureValidation,
    StoreLedgerEvidence, StorePlanError, StoreRuntimeError, StoreWorkError, ValidatedStoreSuccess,
};
use thiserror::Error;

use crate::entity::{AuthoredTransform, AuthoredTransformKind};
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};
use crate::math::{Mat4, Quat, Vec3};
use crate::{SCENE_VALIDATION_POLL_INTERVAL, SceneValidationLimits};

use super::super::production::{
    NodeAuthoringBatch, NodeAuthoringFailure, OperationAuthoringControl,
    OperationAuthoringControlError, Presence, ValidatedNodeBatch, validate_node_batch,
};
use super::super::schema::{
    CanonicalBlockRef, NODE_SCENE_CHILD_REFERENCE_FIELD_COUNT, NODE_SCENE_DIRECTORY_FIELD_COUNT,
    NODE_SCENE_EVIDENCE_FIELD_COUNT, NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE,
    NODE_SCENE_MATRIX_SCALAR_COUNT, NODE_SCENE_NODE_FIELD_COUNT, NODE_SCENE_SCHEMA_REVISION,
    NODE_SCENE_TRANSFORM_MATRIX, NODE_SCENE_TRANSFORM_SYNTHETIC, NODE_SCENE_TRANSFORM_TRS,
    NODE_SCENE_TRS_SCALAR_COUNT, NodeSceneAuthoredTransformSchema,
    NodeSceneChildReferenceTableSchema, NodeSceneDirectorySchema, NodeSceneNodeTableSchema,
    NodeSceneOutcomeEvidenceSchema, PRESENCE_ABSENT, PRESENCE_PRESENT,
    node_scene_child_reference_field, node_scene_directory_field, node_scene_node_field,
};
use super::super::validation::{validate_node_scene_evidence_graph, validate_node_scene_graph};
use super::{
    ConstructionBrand, FALLBACK_CODE, GENERATION_EXHAUSTED_CODE, PreparedTracerSceneFallback,
    SceneConstructionError, SceneConstructionLineage, SceneConstructionOperation,
    SceneConstructionOwner, SceneConstructionPhase, SceneConstructionUnit, SceneStoreAssembly,
    SceneStoreKeys, SceneStoreOperation, TracerSceneFallback, burn_generation_reservation,
    final_progress,
};

mod materialization;
mod publication;
mod view;

#[cfg(test)]
mod tests;

const NODE_SCENE_BUILD_INVARIANT_CODE: u32 = 0x5343_1001;
const NODE_SCENE_CONTROL_INTERRUPTION_CODE: u32 = 0x5343_1002;
const NODE_SCENE_WORK_INTERRUPTION_CODE: u32 = 0x5343_1003;
const NODE_SCENE_COMMITMENT_REJECTION_CODE: u32 = 0x5343_1004;
const NODE_SCENE_STORAGE_REJECTION_CODE: u32 = 0x5343_1005;

#[derive(Clone, Copy)]
struct NodeSceneBuildFaults {
    alter_first_authored_transform: bool,
    outcome_evidence_commitment: u64,
}

impl NodeSceneBuildFaults {
    const fn none() -> Self {
        Self {
            alter_first_authored_transform: false,
            outcome_evidence_commitment: 1,
        }
    }

    #[cfg(test)]
    const fn altered_authored_transform() -> Self {
        Self {
            alter_first_authored_transform: true,
            ..Self::none()
        }
    }

    #[cfg(test)]
    const fn excess_outcome_evidence_commitment() -> Self {
        Self {
            outcome_evidence_commitment: 2,
            ..Self::none()
        }
    }

    fn transform_scalar(&mut self, value: f32) -> f32 {
        if !self.alter_first_authored_transform {
            return value;
        }
        self.alter_first_authored_transform = false;
        f32::from_bits(value.to_bits() ^ 1)
    }
}

struct NodeSceneWorkMeter {
    started: bool,
    work_since_poll: u64,
}

impl NodeSceneWorkMeter {
    const fn new() -> Self {
        Self {
            started: false,
            work_since_poll: 0,
        }
    }

    fn observe(
        &mut self,
        control: &mut RetainedStoreWorkControl<
            '_,
            SceneConstructionOwner,
            SceneConstructionPhase,
            SceneConstructionUnit,
        >,
        amount: u64,
    ) -> Result<(), StoreWorkError> {
        if amount == 0 {
            return Ok(());
        }
        if !self.started {
            self.poll_now(control)?;
        }
        let mut remaining = amount;
        while remaining != 0 {
            let until_poll = SCENE_VALIDATION_POLL_INTERVAL - self.work_since_poll;
            let chunk = remaining.min(until_poll);
            control.charge_observed(chunk)?;
            self.work_since_poll += chunk;
            remaining -= chunk;
            if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
                self.poll_now(control)?;
            }
        }
        Ok(())
    }

    fn finish(
        &mut self,
        control: &mut RetainedStoreWorkControl<
            '_,
            SceneConstructionOwner,
            SceneConstructionPhase,
            SceneConstructionUnit,
        >,
    ) -> Result<(), StoreWorkError> {
        self.poll_now(control)
    }

    fn poll_now(
        &mut self,
        control: &mut RetainedStoreWorkControl<
            '_,
            SceneConstructionOwner,
            SceneConstructionPhase,
            SceneConstructionUnit,
        >,
    ) -> Result<(), StoreWorkError> {
        control.poll()?;
        self.started = true;
        self.work_since_poll = 0;
        Ok(())
    }
}

type ValidatedNodeStoreSuccess = ValidatedStoreSuccess<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    RecordBlock,
    NodeSceneDirectorySchema,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
>;
type PreparedNodeStoreSuccess = baozi_operation::PreparedStoreSuccess<
    RecordBlock,
    NodeSceneDirectorySchema,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
>;
type NodeSceneStoreFailureAssembly = StoreFailureAssembly<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
>;
type NodeSceneStoreFailureValidation = StoreFailureValidation<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    RecordBlock,
    NodeSceneOutcomeEvidenceSchema,
>;
type PreparedNodeStoreFailure =
    baozi_operation::PreparedStoreFailure<RecordBlock, NodeSceneOutcomeEvidenceSchema>;

struct FreshNodeSceneOwner {
    raw: SceneStoreOperation,
    brand: ConstructionBrand,
    keys: SceneStoreKeys,
}

/// A validated node-domain input coupled to the exact fresh operation that paid for it.
#[must_use = "the validated node Scene construction must be prepared or deliberately dropped"]
pub(in crate::canonical_scene::store) struct ValidatedNodeSceneConstruction<'input> {
    owner: FreshNodeSceneOwner,
    batch: ValidatedNodeBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum NodeSceneConstructionRejection {
    #[error("derived Scene lineage cannot author a fresh node-domain Scene")]
    DerivedLineage,
    #[error("node-domain authoring validation rejected the batch")]
    Authoring(#[source] NodeAuthoringFailure<OperationAuthoringControlError>),
}

/// A consumed validation attempt which can only terminate as minimal fallback.
#[must_use = "a rejected node Scene construction still owns a terminal fallback"]
pub(in crate::canonical_scene::store) struct RejectedNodeSceneConstruction<'scene> {
    operation: SceneConstructionOperation<'scene>,
    failure: NodeSceneConstructionRejection,
}

#[derive(Clone, Copy)]
struct NodeSceneBlockRefs {
    directory: CanonicalBlockRef<NodeSceneDirectorySchema>,
    nodes: CanonicalBlockRef<NodeSceneNodeTableSchema>,
    references: Option<CanonicalBlockRef<NodeSceneChildReferenceTableSchema>>,
    transforms: Option<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>>,
}

struct NodeSceneDraft {
    brand: ConstructionBrand,
    directory: CanonicalBlockRef<NodeSceneDirectorySchema>,
}

struct NodeSceneCandidateOwner {
    assembly: SceneStoreAssembly,
    brand: ConstructionBrand,
    keys: SceneStoreKeys,
}

struct NodeSceneCandidate {
    owner: NodeSceneCandidateOwner,
    draft: NodeSceneDraft,
    refs: NodeSceneBlockRefs,
    evidence: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    canonical_block_count: u32,
    node_count: u32,
}

struct PendingNodeSceneCommitments;
struct NodeSceneValidationProof;

struct ValidatedNodeSceneCandidate {
    store: ValidatedNodeStoreSuccess,
    refs: NodeSceneBlockRefs,
    canonical_block_count: u32,
    node_count: u32,
    proof: NodeSceneValidationProof,
    pending_commitments: PendingNodeSceneCommitments,
}

pub(in crate::canonical_scene::store) struct PreparedNodeSceneSuccess {
    generation: ReservedSceneGeneration,
    store: PreparedNodeStoreSuccess,
    refs: NodeSceneBlockRefs,
    proof: NodeSceneValidationProof,
}

pub(in crate::canonical_scene::store) struct PreparedNodeSceneRichFailure {
    store: PreparedNodeStoreFailure,
}

#[must_use = "every prepared node Scene owns an exact frozen publication"]
#[allow(
    clippy::large_enum_variant,
    reason = "prepared publication owners remain inline so terminal handoff is allocation-free"
)]
pub(in crate::canonical_scene::store) enum PreparedNodeSceneConstruction {
    Success(PreparedNodeSceneSuccess),
    RichFailure(PreparedNodeSceneRichFailure),
    MinimalFallback(PreparedTracerSceneFallback),
}

/// Private inspector over a centrally validated node-domain Scene.
pub(in crate::canonical_scene::store) struct ValidatedNodeScene {
    generation: SceneGenerationId,
    canonical: PublishedCanonicalStore<RecordBlock, NodeSceneDirectorySchema>,
    refs: NodeSceneBlockRefs,
    _proof: NodeSceneValidationProof,
}

pub(in crate::canonical_scene::store) struct NodeSceneSuccessReport {
    outcome_evidence: PublishedOutcomeEvidenceStore<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
    ledger_evidence: StoreLedgerEvidence,
}

pub(in crate::canonical_scene::store) struct NodeSceneSuccessParts {
    pub(in crate::canonical_scene::store) scene: ValidatedNodeScene,
    pub(in crate::canonical_scene::store) report: NodeSceneSuccessReport,
}

pub(in crate::canonical_scene::store) struct NodeSceneRichFailure {
    outcome_evidence: PublishedRichFailureStore<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
    ledger_evidence: StoreLedgerEvidence,
}

#[must_use = "the terminal owns the exact node Scene construction outcome"]
#[allow(
    clippy::large_enum_variant,
    reason = "published Scene and evidence owners remain inline so handoff is a pure ownership move"
)]
pub(in crate::canonical_scene::store) enum NodeSceneConstructionTerminal {
    Success(NodeSceneSuccessParts),
    RichFailure(NodeSceneRichFailure),
    MinimalFallback(TracerSceneFallback),
}
