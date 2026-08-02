#![allow(
    clippy::result_large_err,
    reason = "retained construction errors preserve bounded fail-closed evidence inline without an unmetered failure-path allocation"
)]

use std::num::{NonZeroU32, NonZeroU64};
use std::sync::atomic::{AtomicU64, Ordering};

use baozi_operation::{
    CanonicalInheritanceSource, DerivedRetainedStoreOperation, DimensionPolicy, FinalProgress,
    FreshStoreLineage, LedgerLimits, LedgerLimitsError, OperationControl, OperationOwner,
    OutcomeEvidenceRecordRef, PhysicalStorageAccount, PhysicalStorageAccountCreateError,
    PhysicalStorageAccountFactory, PhysicalStorageAccountLimits, PreparedStoreFailureOutcome,
    PreparedStoreSuccessOutcome, ProgressPhase, ProgressUnit, PublishedCanonicalStore,
    PublishedOutcomeEvidenceStore, PublishedRichFailureStore, PublishedStoreFallbackOutput,
    RecordBlock, RecordPlan, RetainedStoreBatchLimits, RetainedStoreBlockTablePlan,
    RetainedStoreFailureTerminalWorkPlan, RetainedStoreFailureTerminalWorkRequirement,
    RetainedStoreOpenRecovery, RetainedStoreOperation, RetainedStorePublicationAssembly,
    RetainedStorePublicationPlan, RetainedStoreSharePlan, RetainedStoreStartError,
    RetainedStoreStartPlan, RetainedStoreWriterPlan, SequencePlan, StoreDimensionKey,
    StoreDimensionSemanticId, StoreFailureAssembly, StoreFailureValidation, StoreKeyDomain,
    StoreKeyDomainCreateError, StoreLedgerEvidence, StoreLedgerPlan, StoreLedgerPlanError,
    StorePhaseKey, StorePhaseSemanticId, StorePlanError, StorePrepareError, StoreRuntimeError,
    StoreSemanticCatalogId, StoreSuccessValidation, ValidatedStoreSuccess,
};
use thiserror::Error;

use crate::SceneValidationLimits;
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};
use crate::math::Mat4;

use super::production::{SceneAuthoringBatch, SceneInputEnvelope, SceneInputEnvelopeError};
use super::schema::{
    AuthoredMatrixSchema, CANONICAL_BLOCK_COUNT, CanonicalBlockRef, DIRECTORY_FIELD_COUNT,
    DirectorySchema, EVIDENCE_FIELD_COUNT, EVIDENCE_OUTCOME_CANDIDATE, MATRIX_SCALAR_COUNT,
    MATRIX_SCALARS_PER_NODE, NODE_COUNT, NODE_FIELD_COUNT, NodeTableSchema, OutcomeEvidenceSchema,
    PRESENCE_ABSENT, PRESENCE_PRESENT, REFERENCE_COUNT, REFERENCE_FIELD_COUNT,
    ReferenceTableSchema, SCHEMA_REVISION,
};
use super::validation::{validate_evidence_graph, validate_scene_graph};

pub(super) mod node_scene;

#[cfg(test)]
mod complex;

#[cfg(test)]
pub(super) use complex::{
    COMPLEX_SEMANTIC_TRANSCRIPT_SCHEMA_ID, ComplexFixtureFault, ComplexFixtureTerminal,
    PreparedComplexFixture,
};
const STORE_CATALOG: StoreSemanticCatalogId =
    StoreSemanticCatalogId::new(*b"baozi-core-scene", NonZeroU32::MIN);
const STORE_PHASE: StorePhaseSemanticId = StorePhaseSemanticId::new(STORE_CATALOG, NonZeroU64::MIN);
pub(super) const STORE_DIMENSION: StoreDimensionSemanticId =
    StoreDimensionSemanticId::new(STORE_CATALOG, NonZeroU64::MIN);
pub(super) const FAILURE_TERMINAL_WORK_DIMENSION: StoreDimensionSemanticId =
    StoreDimensionSemanticId::new(STORE_CATALOG, NonZeroU64::new(2).unwrap());
const FALLBACK_CODE: u32 = 0x5343_0001;
const SEMANTIC_REJECTION_CODE: u32 = 0x5343_0002;
const GENERATION_EXHAUSTED_CODE: u32 = 0x5343_0003;
const OPEN_PLAN_REJECTION_CODE: u32 = 0x5343_0004;
const OPEN_RESOURCE_REJECTION_CODE: u32 = 0x5343_0005;
const OPEN_CONTROL_REJECTION_CODE: u32 = 0x5343_0006;
const OPEN_LEDGER_REJECTION_CODE: u32 = 0x5343_0007;
const OPEN_CONTAINMENT_REJECTION_CODE: u32 = 0x5343_0008;
// A publication table itself consumes one attribution slot in addition to its new members.
const MAX_ADMITTED_CANONICAL_NEW_BLOCKS: usize = 27;
const TERMINAL_ATTRIBUTION_SLOTS: usize = MAX_ADMITTED_CANONICAL_NEW_BLOCKS + 1;
const TERMINAL_COMMITMENT_SLOTS: usize = 8;
const FAILURE_TERMINAL_EVIDENCE_MEMBERS: usize = 1;

static NEXT_CONSTRUCTION_BRAND: AtomicU64 = AtomicU64::new(1);

fn failure_terminal_work_requirement()
-> Result<RetainedStoreFailureTerminalWorkRequirement, StorePlanError> {
    RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_members(
        FAILURE_TERMINAL_EVIDENCE_MEMBERS,
    )
}

fn tracer_scene_open_failure_code(cause: &StoreRuntimeError) -> u32 {
    match cause {
        StoreRuntimeError::Plan(_)
        | StoreRuntimeError::BatchCapacity
        | StoreRuntimeError::ForeignPhaseDomain { .. }
        | StoreRuntimeError::ForeignDimensionDomain { .. }
        | StoreRuntimeError::PhaseCompletionRegression { .. }
        | StoreRuntimeError::PhaseCompletionPublicationPending
        | StoreRuntimeError::PhaseCompletionPublicationCompleted
        | StoreRuntimeError::PhaseCompletionCatalogShapeMismatch
        | StoreRuntimeError::SlotLimit { .. }
        | StoreRuntimeError::WrongOutcomeMode
        | StoreRuntimeError::InvalidSourceSlot
        | StoreRuntimeError::InvalidPublicationPlan => OPEN_PLAN_REJECTION_CODE,
        StoreRuntimeError::BatchAllocationFailed
        | StoreRuntimeError::AccountingOverflow
        | StoreRuntimeError::PhysicalByteLimit { .. }
        | StoreRuntimeError::PhysicalAllocationLimit { .. }
        | StoreRuntimeError::AllocationFailed
        | StoreRuntimeError::MembershipExhausted => OPEN_RESOURCE_REJECTION_CODE,
        StoreRuntimeError::Control(_) => OPEN_CONTROL_REJECTION_CODE,
        StoreRuntimeError::Ledger(_) => OPEN_LEDGER_REJECTION_CODE,
        StoreRuntimeError::AccountPoisoned(_)
        | StoreRuntimeError::AccountPoisonedWithRetainedCharge(_)
        | StoreRuntimeError::Build(_)
        | StoreRuntimeError::BuilderPanicked
        | StoreRuntimeError::ConstructionInvariantUnverified
        | StoreRuntimeError::ForeignAccount
        | StoreRuntimeError::StorageDropUnverified
        | StoreRuntimeError::OwnershipUnverified
        | StoreRuntimeError::ChargeReleaseUnverified
        | StoreRuntimeError::SuccessOutcomeRejected
        | StoreRuntimeError::UnverifiedControlledCause
        | StoreRuntimeError::FallbackOnly
        | StoreRuntimeError::Poisoned => OPEN_CONTAINMENT_REJECTION_CODE,
        _ => OPEN_CONTAINMENT_REJECTION_CODE,
    }
}

fn prepare_tracer_scene_open_fallback<L>(
    recovery: SceneStoreOpenRecovery<L>,
) -> Result<baozi_operation::PreparedStoreFallback, SceneConstructionError> {
    Ok(recovery.prepare_minimal_fallback_with(tracer_scene_open_failure_code)?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::canonical_scene::store) enum SceneConstructionPhase {
    Publication,
}

impl ProgressPhase for SceneConstructionPhase {
    fn order(self) -> u32 {
        0
    }

    fn stable_id(self) -> &'static str {
        "canonical-scene-publication"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::canonical_scene::store) enum SceneConstructionUnit {
    Scene,
}

impl ProgressUnit for SceneConstructionUnit {
    fn stable_id(self) -> &'static str {
        "scene"
    }
}

struct SceneConstructionOwner;

impl OperationOwner for SceneConstructionOwner {}

type SceneStoreOperation<L = FreshStoreLineage> = RetainedStoreOperation<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    L,
>;
type SceneStoreOpenRecovery<L = FreshStoreLineage> = RetainedStoreOpenRecovery<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    L,
>;
type DerivedSceneStoreOperation<'scene> = DerivedRetainedStoreOperation<
    'scene,
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
>;
type SceneStoreControlCheckpoint<'a> = baozi_operation::RetainedStoreControlCheckpoint<
    'a,
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
>;
type SceneStoreAssembly<L = FreshStoreLineage> = RetainedStorePublicationAssembly<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    L,
>;
type SceneStoreFailureAssembly = StoreFailureAssembly<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    RecordBlock,
    OutcomeEvidenceSchema,
>;
type ValidatedTracerStoreSuccess = ValidatedStoreSuccess<
    SceneConstructionOwner,
    SceneConstructionPhase,
    SceneConstructionUnit,
    RecordBlock,
    DirectorySchema,
    RecordBlock,
    OutcomeEvidenceSchema,
>;
type PreparedTracerStoreSuccess = baozi_operation::PreparedStoreSuccess<
    RecordBlock,
    DirectorySchema,
    RecordBlock,
    OutcomeEvidenceSchema,
>;
type PreparedTracerStoreFailure =
    baozi_operation::PreparedStoreFailure<RecordBlock, OutcomeEvidenceSchema>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SceneConstructionLimits {
    pub(crate) operation_live_bytes: u64,
    pub(crate) dimension_live_bytes: u64,
    pub(crate) attempted_work: u64,
    pub(crate) consumed_work: u64,
    pub(crate) committed_output: u64,
    pub(crate) published_evidence: u64,
}

#[derive(Debug, Error)]
pub(crate) enum SceneConstructionStartError {
    #[error("canonical Scene input envelope could not be admitted before operation start")]
    InputEnvelope(#[from] SceneInputEnvelopeError),
    #[error("canonical Scene construction-brand space is exhausted")]
    BrandExhausted,
    #[error("a private physical storage account could not be created")]
    Account(#[from] PhysicalStorageAccountCreateError),
    #[error("the canonical Scene store key domain could not be created")]
    KeyDomain(#[from] StoreKeyDomainCreateError),
    #[error("the canonical Scene store semantic key could not be admitted")]
    KeyAdmissionInvariant,
    #[error("the canonical Scene work budget cannot cover its bounded terminal reserve")]
    TerminalWorkBudget,
    #[error("canonical Scene ledger limits are invalid")]
    LedgerLimits(#[from] LedgerLimitsError<StoreDimensionKey>),
    #[error("the canonical Scene store ledger plan is invalid")]
    LedgerPlan(#[from] StoreLedgerPlanError),
    #[error("the canonical Scene retained-store plan is invalid")]
    StorePlan(#[from] StorePlanError),
    #[error("the canonical Scene retained-store operation could not start")]
    StoreStart(#[from] RetainedStoreStartError),
}

#[derive(Debug, Error)]
pub(crate) enum SceneConstructionError {
    #[error("the canonical Scene retained-store publication plan is invalid")]
    StorePlan(#[from] StorePlanError),
    #[error("canonical Scene retained-store construction failed")]
    Store(#[from] StoreRuntimeError),
    #[error("canonical Scene terminal preparation failed closed")]
    Prepare(#[from] StorePrepareError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ConstructionBrand(u64);

impl ConstructionBrand {
    fn reserve() -> Result<Self, SceneConstructionStartError> {
        NEXT_CONSTRUCTION_BRAND
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |observed| {
                observed.checked_add(1)
            })
            .map(Self)
            .map_err(|_| SceneConstructionStartError::BrandExhausted)
    }
}

/// A draft is deliberately only an operation brand and a non-owning directory projection.
pub(super) struct SceneDraft {
    pub(super) brand: ConstructionBrand,
    pub(super) directory: CanonicalBlockRef<DirectorySchema>,
}

#[derive(Clone, Copy)]
pub(super) struct SceneBlockRefs {
    pub(super) directory: CanonicalBlockRef<DirectorySchema>,
    pub(super) nodes: CanonicalBlockRef<NodeTableSchema>,
    pub(super) references: CanonicalBlockRef<ReferenceTableSchema>,
    pub(super) matrices: CanonicalBlockRef<AuthoredMatrixSchema>,
}

pub(crate) struct ValidatedTracerScene {
    generation: SceneGenerationId,
    canonical: PublishedCanonicalStore<RecordBlock, DirectorySchema>,
    refs: SceneBlockRefs,
    _proof: TracerSceneValidationProof,
}

struct TracerSceneValidationProof;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OptionalCollectionSnapshot {
    pub(super) presence: u8,
    pub(super) count: u32,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NodeSemanticSnapshot {
    pub(super) node: u32,
    pub(super) parent: OptionalCollectionSnapshot,
    pub(super) parent_reference_offset: u32,
    pub(super) matrix_offset: u32,
    pub(super) matrix_length: u32,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ReferenceSemanticSnapshot {
    pub(super) reference: u32,
    pub(super) parent: u32,
    pub(super) child: u32,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SceneSemanticSnapshot {
    pub(super) schema_revision: u32,
    pub(super) root_node: u32,
    pub(super) node_count: u32,
    pub(super) reference_count: u32,
    pub(super) root_children: OptionalCollectionSnapshot,
    pub(super) child_children: OptionalCollectionSnapshot,
    pub(super) metadata: OptionalCollectionSnapshot,
    pub(super) matrix_scalar_count: u32,
    pub(super) nodes: [NodeSemanticSnapshot; NODE_COUNT],
    pub(super) parent_reference: ReferenceSemanticSnapshot,
    pub(super) authored_matrix_bits: [[u32; MATRIX_SCALARS_PER_NODE]; NODE_COUNT],
}

impl ValidatedTracerScene {
    pub(crate) const fn generation(&self) -> SceneGenerationId {
        self.generation
    }

    pub(crate) fn account(&self) -> &PhysicalStorageAccount {
        self.canonical.account()
    }

    pub(crate) fn authored_matrix_bits(&self, node: usize) -> Option<[u32; 16]> {
        let start = node.checked_mul(super::schema::MATRIX_SCALARS_PER_NODE)?;
        let end = start.checked_add(super::schema::MATRIX_SCALARS_PER_NODE)?;
        let values = self
            .canonical
            .sequence(self.refs.matrices.into_store())?
            .as_slice()
            .get(start..end)?;
        Some(std::array::from_fn(|index| values[index].to_bits()))
    }

    pub(crate) fn authored_matrix(&self, node: usize) -> Option<Mat4> {
        let bits = self.authored_matrix_bits(node)?;
        let values = bits.map(f32::from_bits);
        Some(Mat4 {
            cols: [
                [values[0], values[1], values[2], values[3]],
                [values[4], values[5], values[6], values[7]],
                [values[8], values[9], values[10], values[11]],
                [values[12], values[13], values[14], values[15]],
            ],
        })
    }

    pub(crate) fn canonical_block_count(&self) -> usize {
        self.canonical.len()
    }

    #[cfg(test)]
    pub(super) fn semantic_snapshot(&self) -> Option<SceneSemanticSnapshot> {
        use super::schema::{directory_field, node_field, reference_field};

        let directory = self.canonical.records(self.refs.directory.into_store())?;
        let nodes = self.canonical.records(self.refs.nodes.into_store())?;
        let references = self.canonical.records(self.refs.references.into_store())?;
        let node_snapshot = |row| {
            Some(NodeSemanticSnapshot {
                node: nodes.scalar::<u32>(row, node_field::NODE)?,
                parent: OptionalCollectionSnapshot {
                    presence: nodes.scalar::<u8>(row, node_field::PARENT_PRESENCE)?,
                    count: nodes.scalar::<u32>(row, node_field::PARENT_REFERENCE_COUNT)?,
                },
                parent_reference_offset: nodes
                    .scalar::<u32>(row, node_field::PARENT_REFERENCE_OFFSET)?,
                matrix_offset: nodes.scalar::<u32>(row, node_field::MATRIX_OFFSET)?,
                matrix_length: nodes.scalar::<u32>(row, node_field::MATRIX_LENGTH)?,
            })
        };
        Some(SceneSemanticSnapshot {
            schema_revision: directory.scalar::<u32>(0, directory_field::SCHEMA_REVISION)?,
            root_node: directory.scalar::<u32>(0, directory_field::ROOT_NODE)?,
            node_count: directory.scalar::<u32>(0, directory_field::NODE_COUNT)?,
            reference_count: directory.scalar::<u32>(0, directory_field::REFERENCE_COUNT)?,
            root_children: OptionalCollectionSnapshot {
                presence: directory.scalar::<u8>(0, directory_field::ROOT_CHILDREN_PRESENCE)?,
                count: directory.scalar::<u32>(0, directory_field::ROOT_CHILDREN_COUNT)?,
            },
            child_children: OptionalCollectionSnapshot {
                presence: directory.scalar::<u8>(0, directory_field::CHILD_CHILDREN_PRESENCE)?,
                count: directory.scalar::<u32>(0, directory_field::CHILD_CHILDREN_COUNT)?,
            },
            metadata: OptionalCollectionSnapshot {
                presence: directory.scalar::<u8>(0, directory_field::METADATA_PRESENCE)?,
                count: directory.scalar::<u32>(0, directory_field::METADATA_COUNT)?,
            },
            matrix_scalar_count: directory
                .scalar::<u32>(0, directory_field::MATRIX_SCALAR_COUNT)?,
            nodes: [node_snapshot(0)?, node_snapshot(1)?],
            parent_reference: ReferenceSemanticSnapshot {
                reference: references.scalar::<u32>(0, reference_field::REFERENCE)?,
                parent: references.scalar::<u32>(0, reference_field::PARENT)?,
                child: references.scalar::<u32>(0, reference_field::CHILD)?,
            },
            authored_matrix_bits: [self.authored_matrix_bits(0)?, self.authored_matrix_bits(1)?],
        })
    }

    #[cfg(test)]
    pub(super) const fn block_refs(&self) -> SceneBlockRefs {
        self.refs
    }

    fn inheritance_source(&self) -> TracerInheritanceSource<'_> {
        TracerInheritanceSource {
            exact_input: self,
            authorized_matrix: self.refs.matrices,
        }
    }
}

pub(crate) struct PreparedTracerSceneSuccess {
    generation: ReservedSceneGeneration,
    store: PreparedTracerStoreSuccess,
    refs: SceneBlockRefs,
    proof: TracerSceneValidationProof,
}

pub(crate) struct PreparedTracerSceneRichFailure {
    store: PreparedTracerStoreFailure,
}

pub(crate) struct PreparedTracerSceneFallback {
    store: baozi_operation::PreparedStoreFallback,
}

#[must_use = "every exact canonical Scene terminal owns a frozen retained publication"]
#[allow(
    clippy::large_enum_variant,
    reason = "prepared publication owners stay inline so the post-admission terminal path cannot allocate"
)]
pub(crate) enum PreparedSceneConstruction {
    Success(PreparedTracerSceneSuccess),
    RichFailure(PreparedTracerSceneRichFailure),
    MinimalFallback(PreparedTracerSceneFallback),
}

pub(crate) struct TracerSceneRichFailure {
    outcome_evidence: PublishedRichFailureStore<RecordBlock, OutcomeEvidenceSchema>,
    ledger_evidence: StoreLedgerEvidence,
}

impl TracerSceneRichFailure {
    pub(crate) fn account(&self) -> &PhysicalStorageAccount {
        self.outcome_evidence.account()
    }

    pub(crate) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        &self.ledger_evidence
    }

    pub(crate) fn outcome_evidence_block_count(&self) -> Option<usize> {
        let published = self
            .ledger_evidence
            .dimension_by_semantic(STORE_DIMENSION)?
            .published_outcome_evidence();
        usize::try_from(published).ok()
    }
}

pub(crate) struct TracerSceneFallback {
    fallback: PublishedStoreFallbackOutput,
    ledger_evidence: StoreLedgerEvidence,
}

pub(crate) struct TracerSceneSuccessReport {
    outcome_evidence: PublishedOutcomeEvidenceStore<RecordBlock, OutcomeEvidenceSchema>,
    ledger_evidence: StoreLedgerEvidence,
}

impl TracerSceneSuccessReport {
    pub(crate) fn outcome_evidence_block_count(&self) -> usize {
        self.outcome_evidence.len()
    }

    pub(crate) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        &self.ledger_evidence
    }
}

pub(crate) struct TracerSceneSuccessParts {
    pub(crate) scene: ValidatedTracerScene,
    pub(crate) report: TracerSceneSuccessReport,
}

impl TracerSceneFallback {
    pub(crate) fn account(&self) -> &PhysicalStorageAccount {
        self.fallback.account()
    }

    pub(crate) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        &self.ledger_evidence
    }
}

#[must_use = "the terminal owns the exact published Scene construction outcome"]
#[allow(
    clippy::large_enum_variant,
    reason = "published Scene and evidence owners stay inline so pure handoff cannot allocate"
)]
pub(crate) enum SceneConstructionTerminal {
    Success(TracerSceneSuccessParts),
    RichFailure(TracerSceneRichFailure),
    MinimalFallback(TracerSceneFallback),
}

impl PreparedSceneConstruction {
    pub(crate) fn handoff(self) -> SceneConstructionTerminal {
        match self {
            Self::Success(prepared) => {
                let published = prepared.store.handoff();
                let generation = prepared.generation.into_id();
                SceneConstructionTerminal::Success(TracerSceneSuccessParts {
                    scene: ValidatedTracerScene {
                        generation,
                        canonical: published.canonical,
                        refs: prepared.refs,
                        _proof: prepared.proof,
                    },
                    report: TracerSceneSuccessReport {
                        outcome_evidence: published.outcome_evidence,
                        ledger_evidence: published.ledger_evidence,
                    },
                })
            }
            Self::RichFailure(prepared) => {
                let published = prepared.store.handoff();
                SceneConstructionTerminal::RichFailure(TracerSceneRichFailure {
                    outcome_evidence: published.outcome_evidence,
                    ledger_evidence: published.ledger_evidence,
                })
            }
            Self::MinimalFallback(prepared) => {
                let published = prepared.store.handoff();
                SceneConstructionTerminal::MinimalFallback(TracerSceneFallback {
                    fallback: published.fallback,
                    ledger_evidence: published.ledger_evidence,
                })
            }
        }
    }
}

struct TracerInheritanceSource<'scene> {
    exact_input: &'scene ValidatedTracerScene,
    authorized_matrix: CanonicalBlockRef<AuthoredMatrixSchema>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TwoNodeAuthoredMatrices {
    root: Mat4,
    child: Mat4,
}

impl TwoNodeAuthoredMatrices {
    pub(crate) const fn new(root: Mat4, child: Mat4) -> Self {
        Self { root, child }
    }

    pub(crate) const fn root(self) -> Mat4 {
        self.root
    }

    pub(crate) const fn child(self) -> Mat4 {
        self.child
    }

    fn scalars(self) -> [f32; MATRIX_SCALAR_COUNT] {
        let mut values = [0.0; MATRIX_SCALAR_COUNT];
        for (node, matrix) in [self.root, self.child].into_iter().enumerate() {
            let node_start = node * MATRIX_SCALARS_PER_NODE;
            for (column, values_in_column) in matrix.cols.into_iter().enumerate() {
                let column_start = node_start + column * 4;
                values[column_start..column_start + 4].copy_from_slice(&values_in_column);
            }
        }
        values
    }
}

#[derive(Clone, Copy)]
struct SceneStoreKeys {
    phase: StorePhaseKey,
    dimension: StoreDimensionKey,
    failure_terminal_work_dimension: StoreDimensionKey,
}

struct SceneCandidateOwner<L = FreshStoreLineage> {
    assembly: SceneStoreAssembly<L>,
    brand: ConstructionBrand,
    keys: SceneStoreKeys,
}

struct SceneCandidate<L = FreshStoreLineage> {
    owner: SceneCandidateOwner<L>,
    draft: SceneDraft,
    refs: SceneBlockRefs,
    evidence: OutcomeEvidenceRecordRef<OutcomeEvidenceSchema>,
    outcome_evidence_commitment: u64,
}

/// Proof that bounded destination commitment intents were registered while the assembly was open.
///
/// The concrete-store API deliberately accepts these intents only on the writable assembly. They
/// do not mutate either destination: physical application and ledger freeze happen inside
/// `ValidatedStoreSuccess::try_prepare`, after semantic closure and generation reservation.
struct PendingDestinationCommitments;

/// Semantic/physical closure proof bound to the exact pending destination commitments it validated.
struct ValidatedSceneCandidate {
    store: ValidatedTracerStoreSuccess,
    refs: SceneBlockRefs,
    proof: TracerSceneValidationProof,
    pending_commitments: PendingDestinationCommitments,
}

#[must_use = "the Scene construction operation must be prepared or deliberately dropped"]
pub(crate) struct SceneConstructionOperation {
    raw: SceneStoreOperation,
    brand: ConstructionBrand,
    keys: SceneStoreKeys,
}

#[must_use = "the derived Scene construction operation must be prepared or deliberately dropped"]
pub(crate) struct DerivedSceneConstructionOperation<'scene> {
    raw: DerivedSceneStoreOperation<'scene>,
    brand: ConstructionBrand,
    keys: SceneStoreKeys,
    source: TracerInheritanceSource<'scene>,
}

/// A fresh operation sealed to the exact producer description admitted before startup.
#[must_use = "the admitted Scene input must be validated or deliberately dropped"]
pub(in crate::canonical_scene::store) struct SceneAuthoringOperation<'input> {
    operation: SceneConstructionOperation,
    input: SceneAuthoringBatch<'input>,
    validation_limits: SceneValidationLimits,
}

#[cfg(test)]
pub(super) struct ScaledComplexFixtureOperation {
    operation: SceneConstructionOperation,
    image_count: usize,
}

#[cfg(test)]
impl ScaledComplexFixtureOperation {
    pub(super) fn prepare(
        self,
        fault: Option<ComplexFixtureFault>,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedComplexFixture, SceneConstructionError> {
        complex::prepare(self.operation, self.image_count, fault, reserve_generation)
    }
}

impl SceneConstructionOperation {
    pub(crate) fn with_account(
        account: &PhysicalStorageAccount,
        limits: SceneConstructionLimits,
    ) -> Result<Self, SceneConstructionStartError> {
        Self::start(account, limits)
    }

    pub(in crate::canonical_scene::store) fn with_account_for_scene_input<'input>(
        account: &PhysicalStorageAccount,
        limits: SceneConstructionLimits,
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
    ) -> Result<SceneAuthoringOperation<'input>, SceneConstructionStartError> {
        Self::with_account_and_control_for_scene_input(
            account,
            limits,
            OperationControl::default(),
            input,
            validation_limits,
        )
    }

    pub(in crate::canonical_scene::store) fn with_account_and_control_for_scene_input<'input>(
        account: &PhysicalStorageAccount,
        limits: SceneConstructionLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
    ) -> Result<SceneAuthoringOperation<'input>, SceneConstructionStartError> {
        let envelope = SceneInputEnvelope::try_new(input, validation_limits)?;
        let operation = Self::start_with_control_and_attribution_slots(
            account,
            limits,
            control,
            envelope.terminal_attribution_slots(),
        )?;
        Ok(SceneAuthoringOperation {
            operation,
            input,
            validation_limits,
        })
    }

    #[cfg(test)]
    pub(super) fn with_account_and_cancellation_for_scene_input_for_test<'input>(
        account: &PhysicalStorageAccount,
        limits: SceneConstructionLimits,
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
        cancellation: impl baozi_operation::CancellationSignal,
    ) -> Result<SceneAuthoringOperation<'input>, SceneConstructionStartError> {
        Self::with_account_and_control_for_scene_input(
            account,
            limits,
            OperationControl::default().with_cancellation(cancellation),
            input,
            validation_limits,
        )
    }

    #[cfg(test)]
    pub(super) fn with_account_for_scaled_complex_fixture(
        account: &PhysicalStorageAccount,
        limits: SceneConstructionLimits,
        image_count: usize,
    ) -> Result<ScaledComplexFixtureOperation, SceneConstructionStartError> {
        let attribution_slots = complex::scaled_complex_attribution_slots(image_count)
            .expect("the scaled complex image count must be admitted");
        let operation = Self::start_with_control_and_attribution_slots(
            account,
            limits,
            OperationControl::default(),
            attribution_slots,
        )?;
        Ok(ScaledComplexFixtureOperation {
            operation,
            image_count,
        })
    }

    pub(crate) fn with_private_account(
        factory: &PhysicalStorageAccountFactory,
        account_limits: PhysicalStorageAccountLimits,
        limits: SceneConstructionLimits,
    ) -> Result<Self, SceneConstructionStartError> {
        let account = factory.try_create_account(account_limits)?;
        Self::start(&account, limits)
    }

    #[cfg(test)]
    pub(super) fn with_account_and_cancellation_for_test(
        account: &PhysicalStorageAccount,
        limits: SceneConstructionLimits,
        cancellation: impl baozi_operation::CancellationSignal,
    ) -> Result<Self, SceneConstructionStartError> {
        Self::start_with_control(
            account,
            limits,
            OperationControl::default().with_cancellation(cancellation),
        )
    }

    #[cfg(test)]
    pub(super) fn with_account_and_panicking_progress_observer_for_test(
        account: &PhysicalStorageAccount,
        limits: SceneConstructionLimits,
    ) -> Result<Self, SceneConstructionStartError> {
        Self::start_with_control(
            account,
            limits,
            OperationControl::default().with_progress_observer(|_| {
                panic!("test progress observer panic during terminal preparation")
            }),
        )
    }

    pub(crate) fn prepare_fresh_two_node(
        self,
        authored: TwoNodeAuthoredMatrices,
    ) -> Result<PreparedSceneConstruction, SceneConstructionError> {
        self.prepare_fresh_two_node_with(
            authored,
            SceneBuildFaults::none(),
            SceneGenerationId::reserve,
        )
    }

    #[cfg(test)]
    pub(super) fn prepare_complex_fixture_for_test(
        self,
        fault: Option<ComplexFixtureFault>,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedComplexFixture, SceneConstructionError> {
        complex::prepare(self, 1, fault, reserve_generation)
    }

    pub(crate) fn derived_from_exact_input<'scene>(
        input: &'scene ValidatedTracerScene,
        limits: SceneConstructionLimits,
    ) -> Result<DerivedSceneConstructionOperation<'scene>, SceneConstructionStartError> {
        let source = input.inheritance_source();
        let start = try_plan_scene_operation(
            limits,
            OperationControl::default(),
            TERMINAL_ATTRIBUTION_SLOTS,
        )?;
        let raw = SceneStoreOperation::try_start_derived(
            start.control,
            start.ledger_plan,
            &input.canonical,
            start.start_plan,
        )?;
        Ok(DerivedSceneConstructionOperation {
            raw,
            brand: start.brand,
            keys: start.keys,
            source,
        })
    }

    #[cfg(test)]
    pub(super) fn prepare_fresh_two_node_with_generation_source(
        self,
        authored: TwoNodeAuthoredMatrices,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedSceneConstruction, SceneConstructionError> {
        self.prepare_fresh_two_node_with(authored, SceneBuildFaults::none(), reserve_generation)
    }

    #[cfg(test)]
    pub(super) fn prepare_fresh_two_node_with_fault_for_test(
        self,
        authored: TwoNodeAuthoredMatrices,
        fault: TestSceneFault,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedSceneConstruction, SceneConstructionError> {
        self.prepare_fresh_two_node_with(
            authored,
            SceneBuildFaults::from_test_fault(fault),
            reserve_generation,
        )
    }

    #[cfg(test)]
    pub(super) fn prepare_fresh_two_node_with_post_reservation_fault_for_test(
        self,
        authored: TwoNodeAuthoredMatrices,
        fault: TestPostReservationFault,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedSceneConstruction, SceneConstructionError> {
        self.prepare_fresh_two_node_with(
            authored,
            SceneBuildFaults::with_post_reservation_fault(fault),
            reserve_generation,
        )
    }

    fn start(
        account: &PhysicalStorageAccount,
        limits: SceneConstructionLimits,
    ) -> Result<Self, SceneConstructionStartError> {
        Self::start_with_control(account, limits, OperationControl::default())
    }

    fn start_with_control(
        account: &PhysicalStorageAccount,
        limits: SceneConstructionLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
    ) -> Result<Self, SceneConstructionStartError> {
        Self::start_with_control_and_attribution_slots(
            account,
            limits,
            control,
            TERMINAL_ATTRIBUTION_SLOTS,
        )
    }

    fn start_with_control_and_attribution_slots(
        account: &PhysicalStorageAccount,
        limits: SceneConstructionLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
        terminal_attribution_slots: usize,
    ) -> Result<Self, SceneConstructionStartError> {
        let start = try_plan_scene_operation(limits, control, terminal_attribution_slots)?;
        let raw = SceneStoreOperation::try_start(
            start.control,
            start.ledger_plan,
            account,
            start.start_plan,
        )?;
        Ok(Self {
            raw,
            brand: start.brand,
            keys: start.keys,
        })
    }

    fn prepare_fresh_two_node_with(
        self,
        authored: TwoNodeAuthoredMatrices,
        faults: SceneBuildFaults,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedSceneConstruction, SceneConstructionError> {
        let Self { raw, brand, keys } = self;
        let mut values = authored.scalars();
        prepare_two_node(
            raw,
            brand,
            keys,
            faults,
            reserve_generation,
            move |assembly| {
                if faults.invalid_matrix() {
                    values[3] = 1.0;
                }
                retain_authored_matrices(assembly, &values)
                    .map(|matrix| (matrix, matrix))
                    .map_err(|_| MatrixConstructionFailure::Storage)
            },
        )
    }
}

impl DerivedSceneConstructionOperation<'_> {
    pub(crate) fn prepare_derived_two_node(
        self,
    ) -> Result<PreparedSceneConstruction, SceneConstructionError> {
        self.prepare_derived_two_node_with(
            None,
            None,
            SceneBuildFaults::none(),
            SceneGenerationId::reserve,
        )
    }

    #[cfg(test)]
    pub(super) fn prepare_derived_two_node_with_generation_source(
        self,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedSceneConstruction, SceneConstructionError> {
        self.prepare_derived_two_node_with(None, None, SceneBuildFaults::none(), reserve_generation)
    }

    #[cfg(test)]
    pub(super) fn prepare_derived_two_node_with_fault_for_test(
        self,
        requested_scene: &ValidatedTracerScene,
        requested_matrix: CanonicalBlockRef<AuthoredMatrixSchema>,
        fault: Option<TestSceneFault>,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedSceneConstruction, SceneConstructionError> {
        self.prepare_derived_two_node_with(
            Some(requested_scene),
            Some(requested_matrix),
            fault.map_or_else(SceneBuildFaults::none, SceneBuildFaults::from_test_fault),
            reserve_generation,
        )
    }

    fn prepare_derived_two_node_with(
        self,
        requested_scene: Option<&ValidatedTracerScene>,
        requested_matrix: Option<CanonicalBlockRef<AuthoredMatrixSchema>>,
        faults: SceneBuildFaults,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedSceneConstruction, SceneConstructionError> {
        let Self {
            raw,
            brand,
            keys,
            source,
        } = self;
        prepare_two_node(
            raw,
            brand,
            keys,
            faults,
            reserve_generation,
            move |assembly| {
                let requested_scene = requested_scene.unwrap_or(source.exact_input);
                let requested_matrix = requested_matrix.unwrap_or(source.authorized_matrix);
                if !source.authorizes(requested_scene, requested_matrix) {
                    return Err(MatrixConstructionFailure::Semantic);
                }
                assembly
                    .try_bind_canonical_source(keys.dimension)
                    .map_err(|_| MatrixConstructionFailure::Storage)?;
                let target_matrix = assembly
                    .try_share_canonical(requested_matrix.into_store())
                    .map(CanonicalBlockRef::from_store)
                    .map_err(|_| MatrixConstructionFailure::Storage)?;
                let directory_matrix = if faults.source_slot_in_directory() {
                    requested_matrix
                } else {
                    target_matrix
                };
                Ok((target_matrix, directory_matrix))
            },
        )
    }
}

struct SceneOperationStart {
    brand: ConstructionBrand,
    keys: SceneStoreKeys,
    control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
    ledger_plan: StoreLedgerPlan,
    start_plan: RetainedStoreStartPlan,
}

fn try_plan_scene_operation(
    limits: SceneConstructionLimits,
    control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
    terminal_attribution_slots: usize,
) -> Result<SceneOperationStart, SceneConstructionStartError> {
    let brand = ConstructionBrand::reserve()?;
    let domain = StoreKeyDomain::try_new(STORE_CATALOG)?;
    let keys = SceneStoreKeys {
        phase: domain
            .try_admit_phase(STORE_PHASE)
            .ok_or(SceneConstructionStartError::KeyAdmissionInvariant)?,
        dimension: domain
            .try_admit_dimension(STORE_DIMENSION)
            .ok_or(SceneConstructionStartError::KeyAdmissionInvariant)?,
        failure_terminal_work_dimension: domain
            .try_admit_dimension(FAILURE_TERMINAL_WORK_DIMENSION)
            .ok_or(SceneConstructionStartError::KeyAdmissionInvariant)?,
    };
    let failure_terminal_work = RetainedStoreFailureTerminalWorkPlan::new(
        keys.failure_terminal_work_dimension,
        FAILURE_TERMINAL_EVIDENCE_MEMBERS,
        limits.attempted_work,
        limits.consumed_work,
    )
    .map_err(|cause| match cause {
        StorePlanError::InvalidFailureTerminalWork => {
            SceneConstructionStartError::TerminalWorkBudget
        }
        cause => SceneConstructionStartError::StorePlan(cause),
    })?;
    let dimension_policy = DimensionPolicy::reservable_bytes(
        failure_terminal_work.ordinary_attempted_work_limit(),
        failure_terminal_work.ordinary_consumed_work_limit(),
        limits.committed_output,
        limits.dimension_live_bytes,
    )
    .with_published_outcome_evidence_limit(limits.published_evidence);
    let ledger_limits = LedgerLimits::new(
        limits.operation_live_bytes,
        [
            (keys.dimension, dimension_policy),
            (
                keys.failure_terminal_work_dimension,
                failure_terminal_work.requirement().dimension_policy(),
            ),
        ],
    )?;
    let ledger_plan = StoreLedgerPlan::try_new(domain, ledger_limits, &[keys.phase])?;
    let terminal_limits =
        RetainedStoreBatchLimits::new(terminal_attribution_slots, TERMINAL_COMMITMENT_SLOTS)?;
    let terminal_writer =
        RetainedStoreWriterPlan::new(keys.phase, keys.dimension, keys.dimension, terminal_limits);
    let start_plan = RetainedStoreStartPlan::new(
        terminal_writer,
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        baozi_operation::StoreFallbackEnvelope::new(FALLBACK_CODE),
        keys.dimension,
        1,
        keys.dimension,
        1,
        failure_terminal_work,
    );
    Ok(SceneOperationStart {
        brand,
        keys,
        control,
        ledger_plan,
        start_plan,
    })
}

enum MatrixConstructionFailure {
    Semantic,
    Storage,
}

trait ScenePublicationLineage {
    const INHERITS_CANONICAL_STORAGE: bool;
}

impl ScenePublicationLineage for FreshStoreLineage {
    const INHERITS_CANONICAL_STORAGE: bool = false;
}

impl ScenePublicationLineage for CanonicalInheritanceSource<'_> {
    const INHERITS_CANONICAL_STORAGE: bool = true;
}

fn prepare_two_node<L: ScenePublicationLineage>(
    raw: SceneStoreOperation<L>,
    brand: ConstructionBrand,
    keys: SceneStoreKeys,
    faults: SceneBuildFaults,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    construct_matrix: impl FnOnce(
        &mut SceneStoreAssembly<L>,
    ) -> Result<
        (
            CanonicalBlockRef<AuthoredMatrixSchema>,
            CanonicalBlockRef<AuthoredMatrixSchema>,
        ),
        MatrixConstructionFailure,
    >,
) -> Result<PreparedSceneConstruction, SceneConstructionError> {
    let publication_plan = scene_publication_plan(keys, L::INHERITS_CANONICAL_STORAGE, faults)?;
    let mut assembly = match raw.try_publication_assembly(publication_plan) {
        Ok(assembly) => assembly,
        Err(error) => {
            let recovery = error.into_disposition()?;
            let prepared = prepare_tracer_scene_open_fallback(recovery)?;
            return Ok(PreparedSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store: prepared },
            ));
        }
    };

    let evidence = match retain_outcome_evidence(&mut assembly) {
        Ok(evidence) => evidence,
        Err(_) => return prepare_minimal_fallback_from_assembly(assembly),
    };

    let (matrix, directory_matrix) = match construct_matrix(&mut assembly) {
        Ok(matrix) => matrix,
        Err(MatrixConstructionFailure::Semantic) => {
            return prepare_rich_failure_from_assembly(assembly, evidence, SEMANTIC_REJECTION_CODE);
        }
        Err(MatrixConstructionFailure::Storage) => {
            return prepare_minimal_fallback_from_assembly(assembly);
        }
    };

    if faults.orphan()
        && assembly
            .try_retain_canonical_sequence(
                SequencePlan::<u8>::new(1).map_err(StoreRuntimeError::Plan)?,
                OrphanSchema,
                |writer| writer.push(1),
            )
            .is_err()
    {
        return prepare_minimal_fallback_from_assembly(assembly);
    }

    let references = match retain_references(&mut assembly, faults) {
        Ok(references) => references,
        Err(_) => return prepare_minimal_fallback_from_assembly(assembly),
    };
    let nodes = match retain_nodes(&mut assembly, matrix, faults) {
        Ok(nodes) => nodes,
        Err(_) => return prepare_minimal_fallback_from_assembly(assembly),
    };
    let directory =
        match retain_directory(&mut assembly, nodes, references, directory_matrix, faults) {
            Ok(directory) => directory,
            Err(_) => return prepare_minimal_fallback_from_assembly(assembly),
        };
    let refs = SceneBlockRefs {
        directory,
        nodes,
        references,
        matrices: matrix,
    };
    let candidate = SceneCandidate {
        owner: SceneCandidateOwner {
            assembly,
            brand,
            keys,
        },
        draft: SceneDraft { brand, directory },
        refs,
        evidence,
        outcome_evidence_commitment: faults.outcome_evidence_commitment(),
    };
    prepare_scene_candidate(candidate, faults.post_reservation(), reserve_generation)
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TestSceneFault {
    WrongRoot,
    WrongParent,
    WrongCardinality,
    InvalidMatrix,
    WrongReference,
    OrphanCanonicalBlock,
    SourceSlotInDirectory,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TestPostReservationFault {
    RichPreparationLedgerFailure,
}

#[derive(Clone, Copy)]
enum PostReservationAction {
    Prepare,
    #[cfg(test)]
    DowngradeForRichPreparation,
}

#[derive(Clone, Copy)]
struct SceneBuildFaults {
    #[cfg(test)]
    wrong_root: bool,
    #[cfg(test)]
    wrong_parent: bool,
    #[cfg(test)]
    wrong_cardinality: bool,
    #[cfg(test)]
    invalid_matrix: bool,
    #[cfg(test)]
    wrong_reference: bool,
    #[cfg(test)]
    orphan: bool,
    #[cfg(test)]
    source_slot_in_directory: bool,
    #[cfg(test)]
    outcome_evidence_commitment: u64,
    #[cfg(test)]
    post_reservation: PostReservationAction,
}

impl SceneBuildFaults {
    const fn none() -> Self {
        Self {
            #[cfg(test)]
            wrong_root: false,
            #[cfg(test)]
            wrong_parent: false,
            #[cfg(test)]
            wrong_cardinality: false,
            #[cfg(test)]
            invalid_matrix: false,
            #[cfg(test)]
            wrong_reference: false,
            #[cfg(test)]
            orphan: false,
            #[cfg(test)]
            source_slot_in_directory: false,
            #[cfg(test)]
            outcome_evidence_commitment: 1,
            #[cfg(test)]
            post_reservation: PostReservationAction::Prepare,
        }
    }

    const fn wrong_root(self) -> bool {
        #[cfg(test)]
        return self.wrong_root;
        #[cfg(not(test))]
        false
    }

    const fn wrong_parent(self) -> bool {
        #[cfg(test)]
        return self.wrong_parent;
        #[cfg(not(test))]
        false
    }

    const fn wrong_cardinality(self) -> bool {
        #[cfg(test)]
        return self.wrong_cardinality;
        #[cfg(not(test))]
        false
    }

    const fn invalid_matrix(self) -> bool {
        #[cfg(test)]
        return self.invalid_matrix;
        #[cfg(not(test))]
        false
    }

    const fn wrong_reference(self) -> bool {
        #[cfg(test)]
        return self.wrong_reference;
        #[cfg(not(test))]
        false
    }

    const fn orphan(self) -> bool {
        #[cfg(test)]
        return self.orphan;
        #[cfg(not(test))]
        false
    }

    const fn source_slot_in_directory(self) -> bool {
        #[cfg(test)]
        return self.source_slot_in_directory;
        #[cfg(not(test))]
        false
    }

    const fn outcome_evidence_commitment(self) -> u64 {
        #[cfg(test)]
        return self.outcome_evidence_commitment;
        #[cfg(not(test))]
        1
    }

    const fn post_reservation(self) -> PostReservationAction {
        #[cfg(test)]
        return self.post_reservation;
        #[cfg(not(test))]
        PostReservationAction::Prepare
    }

    #[cfg(test)]
    const fn from_test_fault(fault: TestSceneFault) -> Self {
        let mut faults = Self::none();
        match fault {
            TestSceneFault::WrongRoot => faults.wrong_root = true,
            TestSceneFault::WrongParent => faults.wrong_parent = true,
            TestSceneFault::WrongCardinality => faults.wrong_cardinality = true,
            TestSceneFault::InvalidMatrix => faults.invalid_matrix = true,
            TestSceneFault::WrongReference => faults.wrong_reference = true,
            TestSceneFault::OrphanCanonicalBlock => faults.orphan = true,
            TestSceneFault::SourceSlotInDirectory => faults.source_slot_in_directory = true,
        }
        faults
    }

    #[cfg(test)]
    const fn with_post_reservation_fault(fault: TestPostReservationFault) -> Self {
        let mut faults = Self::none();
        faults.post_reservation = match fault {
            TestPostReservationFault::RichPreparationLedgerFailure => {
                faults.outcome_evidence_commitment = 2;
                PostReservationAction::DowngradeForRichPreparation
            }
        };
        faults
    }
}

#[derive(Clone, Copy)]
struct OrphanSchema;

impl TracerInheritanceSource<'_> {
    fn authorizes(
        &self,
        source_scene: &ValidatedTracerScene,
        source_matrix: CanonicalBlockRef<AuthoredMatrixSchema>,
    ) -> bool {
        std::ptr::eq(self.exact_input, source_scene)
            && self
                .exact_input
                .account()
                .is_same_account(source_scene.account())
            && self.authorized_matrix == source_matrix
    }
}

fn scene_publication_plan(
    keys: SceneStoreKeys,
    inherited_matrix: bool,
    faults: SceneBuildFaults,
) -> Result<RetainedStorePublicationPlan, StorePlanError> {
    let orphan = faults.orphan();
    let canonical_blocks = 4 + usize::from(orphan);
    let canonical_new_blocks = if inherited_matrix { 3 } else { 4 } + usize::from(orphan);
    let canonical_writer = scene_writer_plan(keys, canonical_new_blocks + 1, 4)?;
    let evidence_writer = scene_writer_plan(keys, 2, 4)?;
    Ok(RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(canonical_writer, canonical_blocks, canonical_new_blocks)?,
        RetainedStoreBlockTablePlan::new(evidence_writer, 1, 1)?,
    ))
}

fn scene_writer_plan(
    keys: SceneStoreKeys,
    attribution_slots: usize,
    commitment_slots: usize,
) -> Result<RetainedStoreWriterPlan, StorePlanError> {
    Ok(RetainedStoreWriterPlan::new(
        keys.phase,
        keys.dimension,
        keys.dimension,
        RetainedStoreBatchLimits::new(attribution_slots, commitment_slots)?,
    ))
}

fn retain_outcome_evidence<L>(
    assembly: &mut SceneStoreAssembly<L>,
) -> Result<OutcomeEvidenceRecordRef<OutcomeEvidenceSchema>, StoreRuntimeError> {
    assembly.try_retain_outcome_evidence_records(
        RecordPlan::new(1, EVIDENCE_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?,
        OutcomeEvidenceSchema,
        |writer| {
            writer.write_row(|row| {
                row.write_scalar(SCHEMA_REVISION)?;
                row.write_scalar(EVIDENCE_OUTCOME_CANDIDATE)?;
                row.write_scalar(CANONICAL_BLOCK_COUNT)?;
                row.write_scalar(NODE_COUNT as u32)
            })
        },
    )
}

fn retain_authored_matrices<L>(
    assembly: &mut SceneStoreAssembly<L>,
    values: &[f32; MATRIX_SCALAR_COUNT],
) -> Result<CanonicalBlockRef<AuthoredMatrixSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence(
            SequencePlan::<f32>::new(MATRIX_SCALAR_COUNT).map_err(StoreRuntimeError::Plan)?,
            AuthoredMatrixSchema,
            |writer| writer.extend_from_slice(values),
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_references<L>(
    assembly: &mut SceneStoreAssembly<L>,
    faults: SceneBuildFaults,
) -> Result<CanonicalBlockRef<ReferenceTableSchema>, StoreRuntimeError> {
    let wrong_reference = faults.wrong_reference();
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(REFERENCE_COUNT, REFERENCE_FIELD_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            ReferenceTableSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u32)?;
                    row.write_scalar(u32::from(!wrong_reference))
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_nodes<L>(
    assembly: &mut SceneStoreAssembly<L>,
    matrices: CanonicalBlockRef<AuthoredMatrixSchema>,
    faults: SceneBuildFaults,
) -> Result<CanonicalBlockRef<NodeTableSchema>, StoreRuntimeError> {
    let wrong_parent = faults.wrong_parent();
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(NODE_COUNT, NODE_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?,
            NodeTableSchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(PRESENCE_ABSENT)?;
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u32)?;
                    row.write_scalar(MATRIX_SCALARS_PER_NODE as u32)?;
                    row.write_sequence_ref(matrices.into_store())
                })?;
                writer.write_row(|row| {
                    row.write_scalar(1_u32)?;
                    row.write_scalar(if wrong_parent {
                        PRESENCE_ABSENT
                    } else {
                        PRESENCE_PRESENT
                    })?;
                    row.write_scalar(0_u32)?;
                    row.write_scalar(u32::from(!wrong_parent))?;
                    row.write_scalar(MATRIX_SCALARS_PER_NODE as u32)?;
                    row.write_scalar(MATRIX_SCALARS_PER_NODE as u32)?;
                    row.write_sequence_ref(matrices.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_directory<L>(
    assembly: &mut SceneStoreAssembly<L>,
    nodes: CanonicalBlockRef<NodeTableSchema>,
    references: CanonicalBlockRef<ReferenceTableSchema>,
    matrices: CanonicalBlockRef<AuthoredMatrixSchema>,
    faults: SceneBuildFaults,
) -> Result<CanonicalBlockRef<DirectorySchema>, StoreRuntimeError> {
    let wrong_root = faults.wrong_root();
    let wrong_cardinality = faults.wrong_cardinality();
    assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, DIRECTORY_FIELD_COUNT).map_err(StoreRuntimeError::Plan)?,
            DirectorySchema,
            |writer| {
                writer.write_row(|row| {
                    row.write_scalar(SCHEMA_REVISION)?;
                    row.write_scalar(u32::from(wrong_root))?;
                    row.write_scalar(NODE_COUNT as u32 + u32::from(wrong_cardinality))?;
                    row.write_record_ref(nodes.into_store())?;
                    row.write_scalar(REFERENCE_COUNT as u32)?;
                    row.write_record_ref(references.into_store())?;
                    row.write_scalar(PRESENCE_PRESENT)?;
                    row.write_scalar(1_u32)?;
                    row.write_scalar(PRESENCE_PRESENT)?;
                    row.write_scalar(0_u32)?;
                    row.write_scalar(PRESENCE_ABSENT)?;
                    row.write_scalar(0_u32)?;
                    row.write_scalar(MATRIX_SCALAR_COUNT as u32)?;
                    row.write_sequence_ref(matrices.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn prepare_scene_candidate<L>(
    candidate: SceneCandidate<L>,
    post_reservation: PostReservationAction,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedSceneConstruction, SceneConstructionError> {
    let SceneCandidate {
        owner:
            SceneCandidateOwner {
                mut assembly,
                brand,
                keys,
            },
        draft,
        refs,
        evidence,
        outcome_evidence_commitment,
    } = candidate;
    let commitments = match register_pending_destination_commitments(
        &mut assembly,
        keys,
        outcome_evidence_commitment,
    ) {
        Ok(commitments) => commitments,
        Err(_) => return prepare_minimal_fallback_from_assembly(assembly),
    };

    let brand_matches = draft.brand == brand && draft.directory == refs.directory;
    match assembly.validate_success(
        draft.directory.into_store(),
        evidence,
        |session, directory, evidence| {
            brand_matches && validate_scene_graph(session, directory, evidence)
        },
    ) {
        StoreSuccessValidation::Valid(store) => prepare_validated_success(
            ValidatedSceneCandidate {
                store,
                refs,
                proof: TracerSceneValidationProof,
                pending_commitments: commitments,
            },
            post_reservation,
            reserve_generation,
        ),
        StoreSuccessValidation::Rejected(failure) => {
            prepare_rich_failure(failure, SEMANTIC_REJECTION_CODE)
        }
        StoreSuccessValidation::FallbackOnly(fallback) => {
            let prepared = fallback.prepare_minimal_fallback()?;
            Ok(PreparedSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store: prepared },
            ))
        }
    }
}

fn register_pending_destination_commitments<L>(
    assembly: &mut SceneStoreAssembly<L>,
    keys: SceneStoreKeys,
    outcome_evidence: u64,
) -> Result<PendingDestinationCommitments, StoreRuntimeError> {
    assembly.try_commit_canonical(keys.dimension, u64::from(CANONICAL_BLOCK_COUNT))?;
    assembly.try_commit_outcome_evidence(keys.dimension, outcome_evidence)?;
    Ok(PendingDestinationCommitments)
}

fn prepare_validated_success(
    candidate: ValidatedSceneCandidate,
    post_reservation: PostReservationAction,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedSceneConstruction, SceneConstructionError> {
    let ValidatedSceneCandidate {
        store: validated,
        refs,
        proof,
        pending_commitments,
    } = candidate;
    let reservation = match reserve_generation() {
        Ok(reservation) => reservation,
        Err(SceneGenerationError::Exhausted) => {
            return prepare_rich_failure(
                validated.downgrade_to_rich_failure(GENERATION_EXHAUSTED_CODE),
                GENERATION_EXHAUSTED_CODE,
            );
        }
    };
    #[cfg(test)]
    match post_reservation {
        PostReservationAction::DowngradeForRichPreparation => {
            burn_generation_reservation(reservation);
            let failure = validated.downgrade_to_rich_failure(SEMANTIC_REJECTION_CODE);
            return prepare_rich_failure(failure, SEMANTIC_REJECTION_CODE);
        }
        PostReservationAction::Prepare => {}
    }
    #[cfg(not(test))]
    let PostReservationAction::Prepare = post_reservation;
    let PendingDestinationCommitments = pending_commitments;
    // This is the first operation transition that applies the registered destination commitments.
    match validated.try_prepare(final_progress())? {
        PreparedStoreSuccessOutcome::Success(store) => Ok(PreparedSceneConstruction::Success(
            PreparedTracerSceneSuccess {
                generation: reservation,
                store,
                refs,
                proof,
            },
        )),
        PreparedStoreSuccessOutcome::RichFailure(store) => {
            burn_generation_reservation(reservation);
            Ok(PreparedSceneConstruction::RichFailure(
                PreparedTracerSceneRichFailure { store },
            ))
        }
        PreparedStoreSuccessOutcome::MinimalFallback(store) => {
            burn_generation_reservation(reservation);
            Ok(PreparedSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
    }
}

fn burn_generation_reservation(_reservation: ReservedSceneGeneration) {}

fn prepare_rich_failure_from_assembly<L>(
    assembly: SceneStoreAssembly<L>,
    evidence: OutcomeEvidenceRecordRef<OutcomeEvidenceSchema>,
    cause: u32,
) -> Result<PreparedSceneConstruction, SceneConstructionError> {
    match assembly.validate_rich_failure(evidence, validate_evidence_graph) {
        StoreFailureValidation::Valid(validated) => {
            map_prepared_failure(validated.try_prepare(cause, final_progress())?)
        }
        StoreFailureValidation::Rejected(rejected) => {
            let prepared = rejected.prepare_minimal_fallback()?;
            Ok(PreparedSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store: prepared },
            ))
        }
        StoreFailureValidation::FallbackOnly(fallback) => {
            let prepared = fallback.prepare_minimal_fallback()?;
            Ok(PreparedSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store: prepared },
            ))
        }
    }
}

fn prepare_rich_failure(
    failure: SceneStoreFailureAssembly,
    cause: u32,
) -> Result<PreparedSceneConstruction, SceneConstructionError> {
    let evidence = failure.evidence_root();
    match failure.validate_rich_failure(evidence, validate_evidence_graph) {
        StoreFailureValidation::Valid(validated) => {
            map_prepared_failure(validated.try_prepare(cause, final_progress())?)
        }
        StoreFailureValidation::Rejected(rejected) => {
            let prepared = rejected.prepare_minimal_fallback()?;
            Ok(PreparedSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store: prepared },
            ))
        }
        StoreFailureValidation::FallbackOnly(fallback) => {
            let prepared = fallback.prepare_minimal_fallback()?;
            Ok(PreparedSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store: prepared },
            ))
        }
    }
}

fn map_prepared_failure(
    prepared: PreparedStoreFailureOutcome<RecordBlock, OutcomeEvidenceSchema>,
) -> Result<PreparedSceneConstruction, SceneConstructionError> {
    Ok(match prepared {
        PreparedStoreFailureOutcome::RichFailure(store) => {
            PreparedSceneConstruction::RichFailure(PreparedTracerSceneRichFailure { store })
        }
        PreparedStoreFailureOutcome::MinimalFallback(store) => {
            PreparedSceneConstruction::MinimalFallback(PreparedTracerSceneFallback { store })
        }
    })
}

fn prepare_minimal_fallback_from_assembly<L>(
    assembly: SceneStoreAssembly<L>,
) -> Result<PreparedSceneConstruction, SceneConstructionError> {
    let store = assembly.prepare_minimal_fallback()?;
    Ok(PreparedSceneConstruction::MinimalFallback(
        PreparedTracerSceneFallback { store },
    ))
}

const fn final_progress() -> FinalProgress<SceneConstructionPhase, SceneConstructionUnit> {
    FinalProgress::new(
        SceneConstructionPhase::Publication,
        SceneConstructionUnit::Scene,
        1,
    )
}
