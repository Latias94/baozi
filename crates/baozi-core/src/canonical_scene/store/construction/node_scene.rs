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

impl RejectedNodeSceneConstruction<'_> {
    pub(in crate::canonical_scene::store) const fn failure(
        &self,
    ) -> &NodeSceneConstructionRejection {
        &self.failure
    }

    pub(in crate::canonical_scene::store) fn prepare_minimal_fallback(
        self,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        let Self {
            operation:
                SceneConstructionOperation {
                    raw,
                    brand: _,
                    keys: _,
                    lineage: _,
                },
            failure: _,
        } = self;
        let store = raw.prepare_minimal_fallback(FALLBACK_CODE)?;
        Ok(PreparedNodeSceneConstruction::MinimalFallback(
            PreparedTracerSceneFallback { store },
        ))
    }
}

impl<'scene> SceneConstructionOperation<'scene> {
    pub(in crate::canonical_scene::store) fn try_validate_node_batch<'input>(
        mut self,
        input: NodeAuthoringBatch<'input>,
        limits: SceneValidationLimits,
    ) -> Result<ValidatedNodeSceneConstruction<'input>, RejectedNodeSceneConstruction<'scene>> {
        if !matches!(&self.lineage, SceneConstructionLineage::Fresh) {
            return Err(RejectedNodeSceneConstruction {
                operation: self,
                failure: NodeSceneConstructionRejection::DerivedLineage,
            });
        }

        let keys = self.keys;
        let validation = {
            let checkpoint = self.raw.control_checkpoint();
            let mut control =
                OperationAuthoringControl::new(checkpoint, keys.phase, keys.dimension);
            validate_node_batch(input, limits, &mut control)
        };
        match validation {
            Ok(batch) => {
                let SceneConstructionOperation {
                    raw,
                    brand,
                    keys,
                    lineage,
                } = self;
                debug_assert!(matches!(&lineage, SceneConstructionLineage::Fresh));
                Ok(ValidatedNodeSceneConstruction {
                    owner: FreshNodeSceneOwner { raw, brand, keys },
                    batch,
                })
            }
            Err(failure) => Err(RejectedNodeSceneConstruction {
                operation: self,
                failure: NodeSceneConstructionRejection::Authoring(failure),
            }),
        }
    }
}

#[derive(Clone, Copy)]
struct NodeSceneBlockRefs {
    directory: CanonicalBlockRef<NodeSceneDirectorySchema>,
    nodes: CanonicalBlockRef<NodeSceneNodeTableSchema>,
    references: Option<CanonicalBlockRef<NodeSceneChildReferenceTableSchema>>,
    transforms: Option<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>>,
}

#[derive(Clone, Copy)]
struct NodeSceneDirectoryRefs {
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

struct NodeSceneMaterializationPlans {
    evidence: RecordPlan,
    transforms: Option<SequencePlan<f32>>,
    references: Option<RecordPlan>,
    nodes: RecordPlan,
    directory: RecordPlan,
    publication: RetainedStorePublicationPlan,
    canonical_block_count: u32,
    node_count: u32,
    source_node_count: u32,
    source_root_count: u32,
    transform_scalar_count: u32,
    build_invariant: StoreBuildError,
}

impl NodeSceneMaterializationPlans {
    fn new(
        batch: &ValidatedNodeBatch<'_, RetainedStoreReservation<SceneConstructionOwner>>,
        keys: SceneStoreKeys,
    ) -> Result<Self, StorePlanError> {
        let node_count = u32::try_from(batch.canonical_node_count())
            .map_err(|_| StorePlanError::SizeOverflow)?;
        let source_node_count = node_count
            .checked_sub(1)
            .ok_or(StorePlanError::SizeOverflow)?;
        let source_node_count_usize =
            usize::try_from(source_node_count).map_err(|_| StorePlanError::SizeOverflow)?;
        let source_root_count = match batch.input().roots() {
            Presence::Present(roots) => {
                u32::try_from(roots.len()).map_err(|_| StorePlanError::SizeOverflow)?
            }
            Presence::Absent => return Err(StorePlanError::InvalidBlockTable),
        };
        let transform_scalar_count_u64 = batch
            .matrix_scalar_count()
            .checked_add(batch.trs_scalar_count())
            .ok_or(StorePlanError::SizeOverflow)?;
        let transform_scalar_count =
            u32::try_from(transform_scalar_count_u64).map_err(|_| StorePlanError::SizeOverflow)?;
        let transform_scalar_count_usize =
            usize::try_from(transform_scalar_count).map_err(|_| StorePlanError::SizeOverflow)?;

        let evidence = RecordPlan::new(1, NODE_SCENE_EVIDENCE_FIELD_COUNT)?;
        let nodes = RecordPlan::new(
            usize::try_from(node_count).map_err(|_| StorePlanError::SizeOverflow)?,
            NODE_SCENE_NODE_FIELD_COUNT,
        )?;
        let directory = RecordPlan::new(1, NODE_SCENE_DIRECTORY_FIELD_COUNT)?;
        let references = if source_node_count == 0 {
            None
        } else {
            Some(RecordPlan::new(
                source_node_count_usize,
                NODE_SCENE_CHILD_REFERENCE_FIELD_COUNT,
            )?)
        };
        let transforms = if source_node_count == 0 {
            None
        } else {
            Some(SequencePlan::<f32>::new(transform_scalar_count_usize)?)
        };

        let record_block_count = [Some(nodes), Some(directory), references]
            .into_iter()
            .flatten()
            .count();
        let sequence_block_count = [transforms].into_iter().flatten().count();
        let canonical_block_count_usize = record_block_count
            .checked_add(sequence_block_count)
            .ok_or(StorePlanError::SizeOverflow)?;
        let canonical_block_count =
            u32::try_from(canonical_block_count_usize).map_err(|_| StorePlanError::SizeOverflow)?;
        let evidence_block_count = [Some(evidence)].into_iter().flatten().count();

        let canonical_writer = node_scene_writer_plan(
            keys,
            canonical_block_count_usize
                .checked_add(1)
                .ok_or(StorePlanError::InvalidBlockTable)?,
        )?;
        let evidence_writer = node_scene_writer_plan(
            keys,
            evidence_block_count
                .checked_add(1)
                .ok_or(StorePlanError::InvalidBlockTable)?,
        )?;
        let canonical_table = RetainedStoreBlockTablePlan::new(
            canonical_writer,
            canonical_block_count_usize,
            canonical_block_count_usize,
        )?;
        let evidence_table = RetainedStoreBlockTablePlan::new(
            evidence_writer,
            evidence_block_count,
            evidence_block_count,
        )?;
        let publication = RetainedStorePublicationPlan::success(canonical_table, evidence_table);
        let build_invariant = StoreBuildError::domain(NODE_SCENE_BUILD_INVARIANT_CODE)?;

        Ok(Self {
            evidence,
            transforms,
            references,
            nodes,
            directory,
            publication,
            canonical_block_count,
            node_count,
            source_node_count,
            source_root_count,
            transform_scalar_count,
            build_invariant,
        })
    }
}

fn node_scene_writer_plan(
    keys: SceneStoreKeys,
    attribution_slots: usize,
) -> Result<RetainedStoreWriterPlan, StorePlanError> {
    Ok(RetainedStoreWriterPlan::new(
        keys.phase,
        keys.dimension,
        keys.dimension,
        RetainedStoreBatchLimits::new(attribution_slots, 4)?,
    ))
}

impl ValidatedNodeSceneConstruction<'_> {
    /// Materializes only the batch already coupled to this typestate.
    pub(in crate::canonical_scene::store) fn prepare(
        self,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        self.prepare_with_generation_source(SceneGenerationId::reserve)
    }

    fn prepare_with_generation_source(
        self,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        self.prepare_with_generation_source_and_faults(
            reserve_generation,
            NodeSceneBuildFaults::none(),
        )
    }

    #[cfg(test)]
    fn prepare_with_faults(
        self,
        faults: NodeSceneBuildFaults,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        self.prepare_with_generation_source_and_faults(SceneGenerationId::reserve, faults)
    }

    fn prepare_with_generation_source_and_faults(
        self,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
        mut faults: NodeSceneBuildFaults,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        let Self { owner, batch } = self;
        let FreshNodeSceneOwner { raw, brand, keys } = owner;
        let plans = match NodeSceneMaterializationPlans::new(&batch, keys) {
            Ok(plans) => plans,
            Err(_) => {
                drop(batch);
                return prepare_node_minimal_fallback_from_operation(
                    raw,
                    NODE_SCENE_BUILD_INVARIANT_CODE,
                );
            }
        };
        let mut assembly: SceneStoreAssembly = match raw.try_publication_assembly(plans.publication)
        {
            Ok(assembly) => assembly,
            Err(error) => {
                drop(batch);
                let cause = node_scene_failure_code(error.cause());
                let operation = error.try_into_operation()?;
                return prepare_node_minimal_fallback_from_operation(operation, cause);
            }
        };

        let evidence = match retain_node_scene_evidence(&mut assembly, &plans) {
            Ok(evidence) => evidence,
            Err(cause) => {
                drop(batch);
                return prepare_node_minimal_fallback_from_assembly(
                    assembly,
                    node_scene_failure_code(&cause),
                );
            }
        };
        if let Err(_cause) =
            assembly.try_commit_outcome_evidence(keys.dimension, faults.outcome_evidence_commitment)
        {
            drop(batch);
            return prepare_node_minimal_fallback_from_assembly(
                assembly,
                NODE_SCENE_COMMITMENT_REJECTION_CODE,
            );
        }
        let mut work = NodeSceneWorkMeter::new();
        let transforms = match plans.transforms {
            Some(plan) => match retain_node_scene_transforms(
                &mut assembly,
                plan,
                &batch,
                &plans,
                keys,
                &mut work,
                &mut faults,
            ) {
                Ok(transforms) => Some(transforms),
                Err(cause) => {
                    drop(batch);
                    return prepare_node_post_evidence_failure(assembly, evidence, &plans, cause);
                }
            },
            None => None,
        };
        let references = match plans.references {
            Some(plan) => match retain_node_scene_references(
                &mut assembly,
                plan,
                &batch,
                &plans,
                keys,
                &mut work,
            ) {
                Ok(references) => Some(references),
                Err(cause) => {
                    drop(batch);
                    return prepare_node_post_evidence_failure(assembly, evidence, &plans, cause);
                }
            },
            None => None,
        };
        let nodes = match retain_node_scene_nodes(
            &mut assembly,
            plans.nodes,
            transforms,
            &batch,
            &plans,
            keys,
            &mut work,
        ) {
            Ok(nodes) => nodes,
            Err(cause) => {
                drop(batch);
                return prepare_node_post_evidence_failure(assembly, evidence, &plans, cause);
            }
        };
        let directory = match retain_node_scene_directory(
            &mut assembly,
            plans.directory,
            NodeSceneDirectoryRefs {
                nodes,
                references,
                transforms,
            },
            &plans,
            keys,
            &mut work,
        ) {
            Ok(directory) => directory,
            Err(cause) => {
                drop(batch);
                return prepare_node_post_evidence_failure(assembly, evidence, &plans, cause);
            }
        };
        let refs = NodeSceneBlockRefs {
            directory,
            nodes,
            references,
            transforms,
        };

        prepare_node_scene_candidate(
            NodeSceneCandidate {
                owner: NodeSceneCandidateOwner {
                    assembly,
                    brand,
                    keys,
                },
                draft: NodeSceneDraft { brand, directory },
                refs,
                evidence,
                canonical_block_count: plans.canonical_block_count,
                node_count: plans.node_count,
            },
            batch,
            work,
            reserve_generation,
        )
    }
}

fn retain_node_scene_evidence(
    assembly: &mut SceneStoreAssembly,
    plans: &NodeSceneMaterializationPlans,
) -> Result<OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>, StoreRuntimeError> {
    assembly.try_retain_outcome_evidence_records(
        plans.evidence,
        NodeSceneOutcomeEvidenceSchema,
        |writer| {
            writer.write_row(|row| {
                row.write_scalar(NODE_SCENE_SCHEMA_REVISION)?;
                row.write_scalar(NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE)?;
                row.write_scalar(plans.canonical_block_count)?;
                row.write_scalar(plans.node_count)
            })
        },
    )
}

fn retain_node_scene_transforms(
    assembly: &mut SceneStoreAssembly,
    plan: SequencePlan<f32>,
    batch: &ValidatedNodeBatch<'_, RetainedStoreReservation<SceneConstructionOwner>>,
    plans: &NodeSceneMaterializationPlans,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    faults: &mut NodeSceneBuildFaults,
) -> Result<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>, StoreRuntimeError> {
    let input = batch.input();
    let nodes = input.nodes();
    let failure = plans.build_invariant;
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            NodeSceneAuthoredTransformSchema,
            |writer, control| {
                for &local in batch.canonical_order() {
                    let authored = nodes.get(local).ok_or(failure)?.authored_transform();
                    if let Some(matrix) = authored.authored_matrix() {
                        work.observe(
                            control,
                            u64::try_from(NODE_SCENE_MATRIX_SCALAR_COUNT).map_err(|_| failure)?,
                        )?;
                        for mut column in matrix.cols {
                            column[0] = faults.transform_scalar(column[0]);
                            writer.extend_from_slice(&column)?;
                        }
                    } else if let Some(trs) = authored.authored_trs() {
                        work.observe(
                            control,
                            u64::try_from(NODE_SCENE_TRS_SCALAR_COUNT).map_err(|_| failure)?,
                        )?;
                        let translation = trs.translation();
                        let rotation = trs.rotation();
                        let scale = trs.scale();
                        writer.extend_from_slice(&[
                            faults.transform_scalar(translation.x),
                            translation.y,
                            translation.z,
                            rotation.x,
                            rotation.y,
                            rotation.z,
                            rotation.w,
                            scale.x,
                            scale.y,
                            scale.z,
                        ])?;
                    } else {
                        return Err(failure.into());
                    }
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_node_scene_references(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    batch: &ValidatedNodeBatch<'_, RetainedStoreReservation<SceneConstructionOwner>>,
    plans: &NodeSceneMaterializationPlans,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
) -> Result<CanonicalBlockRef<NodeSceneChildReferenceTableSchema>, StoreRuntimeError> {
    let mapping = batch.canonical_index_by_local();
    let failure = plans.build_invariant;
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            NodeSceneChildReferenceTableSchema,
            |writer, control| {
                for (source_row, &local) in batch.canonical_order().iter().enumerate() {
                    work.observe(control, 1)?;
                    let reference = u32::try_from(source_row).map_err(|_| failure)?;
                    let child = *mapping.get(local).ok_or(failure)?;
                    writer.write_row(|row| {
                        row.write_scalar(reference)?;
                        row.write_scalar(child)
                    })?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_node_scene_nodes(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    transforms: Option<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>>,
    batch: &ValidatedNodeBatch<'_, RetainedStoreReservation<SceneConstructionOwner>>,
    plans: &NodeSceneMaterializationPlans,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
) -> Result<CanonicalBlockRef<NodeSceneNodeTableSchema>, StoreRuntimeError> {
    let input = batch.input();
    let authored_nodes = input.nodes();
    let mapping = batch.canonical_index_by_local();
    let failure = plans.build_invariant;
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            NodeSceneNodeTableSchema,
            |writer, control| {
                work.observe(control, 1)?;
                writer.write_row(|row| {
                    row.write_scalar(0_u32)?;
                    row.write_scalar(PRESENCE_PRESENT)?;
                    row.write_scalar(0_u32)?;
                    row.write_scalar(plans.source_root_count)?;
                    row.write_scalar(NODE_SCENE_TRANSFORM_SYNTHETIC)?;
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u32)?;
                    row.write_scalar(0_u8)
                })?;

                if batch.canonical_order().is_empty() {
                    if transforms.is_some()
                        || plans.transform_scalar_count != 0
                        || plans.source_root_count != 0
                    {
                        return Err(failure.into());
                    }
                    return Ok(());
                }
                let transforms = transforms.ok_or(failure)?;
                let mut transform_offset = 0_u32;
                let mut child_reference_offset = plans.source_root_count;
                for &local in batch.canonical_order() {
                    work.observe(control, 1)?;
                    let authored = authored_nodes
                        .get(local)
                        .ok_or(failure)?
                        .authored_transform();
                    let (kind, transform_length) = match authored.kind() {
                        AuthoredTransformKind::Matrix => (
                            NODE_SCENE_TRANSFORM_MATRIX,
                            u32::try_from(NODE_SCENE_MATRIX_SCALAR_COUNT).map_err(|_| failure)?,
                        ),
                        AuthoredTransformKind::Trs => (
                            NODE_SCENE_TRANSFORM_TRS,
                            u32::try_from(NODE_SCENE_TRS_SCALAR_COUNT).map_err(|_| failure)?,
                        ),
                    };
                    let node = *mapping.get(local).ok_or(failure)?;
                    let child_count = batch.child_count_by_local(local).ok_or(failure)?;
                    writer.write_row(|row| {
                        row.write_scalar(node)?;
                        row.write_scalar(PRESENCE_PRESENT)?;
                        row.write_scalar(child_reference_offset)?;
                        row.write_scalar(child_count)?;
                        row.write_scalar(kind)?;
                        row.write_scalar(transform_offset)?;
                        row.write_scalar(transform_length)?;
                        row.write_sequence_ref(transforms.into_store())
                    })?;
                    transform_offset = transform_offset
                        .checked_add(transform_length)
                        .ok_or(failure)?;
                    child_reference_offset = child_reference_offset
                        .checked_add(child_count)
                        .ok_or(failure)?;
                }
                if transform_offset != plans.transform_scalar_count
                    || child_reference_offset != plans.source_node_count
                {
                    return Err(failure.into());
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_node_scene_directory(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    refs: NodeSceneDirectoryRefs,
    plans: &NodeSceneMaterializationPlans,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
) -> Result<CanonicalBlockRef<NodeSceneDirectorySchema>, StoreRuntimeError> {
    let failure = plans.build_invariant;
    let NodeSceneDirectoryRefs {
        nodes,
        references,
        transforms,
    } = refs;
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            NodeSceneDirectorySchema,
            |writer, control| {
                work.observe(control, 1)?;
                writer
                    .write_row(|row| {
                        row.write_scalar(NODE_SCENE_SCHEMA_REVISION)?;
                        row.write_scalar(0_u32)?;
                        row.write_scalar(plans.node_count)?;
                        row.write_record_ref(nodes.into_store())?;
                        if plans.source_node_count == 0 {
                            if references.is_some() || transforms.is_some() {
                                return Err(failure);
                            }
                            row.write_scalar(PRESENCE_ABSENT)?;
                            row.write_scalar(0_u32)?;
                            row.write_scalar(0_u8)?;
                            row.write_scalar(PRESENCE_ABSENT)?;
                            row.write_scalar(0_u32)?;
                            row.write_scalar(0_u8)?;
                        } else {
                            let references = references.ok_or(failure)?;
                            let transforms = transforms.ok_or(failure)?;
                            row.write_scalar(PRESENCE_PRESENT)?;
                            row.write_scalar(plans.source_node_count)?;
                            row.write_record_ref(references.into_store())?;
                            row.write_scalar(PRESENCE_PRESENT)?;
                            row.write_scalar(plans.transform_scalar_count)?;
                            row.write_sequence_ref(transforms.into_store())?;
                        }
                        row.write_scalar(plans.source_root_count)
                    })
                    .map_err(StoreControlledError::from)
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn prepare_node_scene_candidate<'input>(
    candidate: NodeSceneCandidate,
    batch: ValidatedNodeBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
    mut work: NodeSceneWorkMeter,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    let NodeSceneCandidate {
        owner:
            NodeSceneCandidateOwner {
                mut assembly,
                brand,
                keys,
            },
        draft,
        refs,
        evidence,
        canonical_block_count,
        node_count,
    } = candidate;
    let pending_commitments = match register_node_scene_canonical_commitment(
        &mut assembly,
        keys,
        canonical_block_count,
    ) {
        Ok(commitments) => commitments,
        Err(_) => {
            drop(batch);
            return prepare_node_rich_failure_from_assembly(
                assembly,
                evidence,
                canonical_block_count,
                node_count,
                NODE_SCENE_COMMITMENT_REJECTION_CODE,
            );
        }
    };
    let brand_matches = draft.brand == brand && draft.directory == refs.directory;
    let validation = assembly.validate_success_controlled(
        keys.dimension,
        draft.directory.into_store(),
        evidence,
        |session, directory, evidence, control| {
            let accepted = if brand_matches {
                validate_node_scene_graph(session, directory, evidence, &batch, |amount| {
                    work.observe(control, amount)
                })?
            } else {
                false
            };
            work.finish(control)?;
            Ok(accepted)
        },
    );
    // The central validator is the final reader of the compiled plan and its reservation.
    drop(batch);
    match validation {
        StoreControlledSuccessValidation::InvalidWorkDimension { assembly, cause } => {
            prepare_node_minimal_fallback_from_assembly(assembly, node_scene_failure_code(&cause))
        }
        StoreControlledSuccessValidation::Valid(store) => prepare_validated_node_scene(
            ValidatedNodeSceneCandidate {
                store,
                refs,
                canonical_block_count,
                node_count,
                proof: NodeSceneValidationProof,
                pending_commitments,
            },
            reserve_generation,
        ),
        StoreControlledSuccessValidation::Rejected(failure) => prepare_node_rich_failure(
            failure,
            canonical_block_count,
            node_count,
            NODE_SCENE_BUILD_INVARIANT_CODE,
        ),
        StoreControlledSuccessValidation::Interrupted { failure, cause } => {
            prepare_node_rich_failure(
                failure,
                canonical_block_count,
                node_count,
                controlled_interruption_code(&cause),
            )
        }
        StoreControlledSuccessValidation::FallbackOnly(fallback) => {
            let store = fallback.prepare_minimal_fallback()?;
            Ok(PreparedNodeSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
    }
}

fn post_evidence_failure_code(cause: &StoreRuntimeError) -> Option<u32> {
    match cause {
        StoreRuntimeError::Control(_) => Some(NODE_SCENE_CONTROL_INTERRUPTION_CODE),
        StoreRuntimeError::Build(cause) => post_evidence_build_failure_code(cause.class()),
        StoreRuntimeError::Ledger(
            LedgerCause::LimitExceeded {
                class: CounterClass::Attempted | CounterClass::Consumed,
                ..
            }
            | LedgerCause::ArithmeticOverflow {
                class: CounterClass::Attempted | CounterClass::Consumed,
                ..
            }
            | LedgerCause::ConsumedExceedsAttempted { .. },
        ) => Some(NODE_SCENE_WORK_INTERRUPTION_CODE),
        StoreRuntimeError::Ledger(
            LedgerCause::LimitExceeded {
                class:
                    CounterClass::Live
                    | CounterClass::PendingPublication
                    | CounterClass::Committed
                    | CounterClass::PublishedOutcomeEvidence,
                ..
            }
            | LedgerCause::ArithmeticOverflow {
                class:
                    CounterClass::Live
                    | CounterClass::PendingPublication
                    | CounterClass::Committed
                    | CounterClass::PublishedOutcomeEvidence,
                ..
            },
        )
        | StoreRuntimeError::BatchAllocationFailed
        | StoreRuntimeError::PhysicalByteLimit { .. }
        | StoreRuntimeError::PhysicalAllocationLimit { .. }
        | StoreRuntimeError::AllocationFailed
        | StoreRuntimeError::MembershipExhausted => Some(NODE_SCENE_STORAGE_REJECTION_CODE),
        _ => None,
    }
}

const fn post_evidence_build_failure_code(class: StoreBuildFailureClass) -> Option<u32> {
    match class {
        StoreBuildFailureClass::AllocationFailed => Some(NODE_SCENE_STORAGE_REJECTION_CODE),
        _ => None,
    }
}

fn node_scene_failure_code(cause: &StoreRuntimeError) -> u32 {
    post_evidence_failure_code(cause).unwrap_or(NODE_SCENE_BUILD_INVARIANT_CODE)
}

fn controlled_interruption_code(cause: &StoreWorkError) -> u32 {
    if cause.control_cause().is_some() {
        NODE_SCENE_CONTROL_INTERRUPTION_CODE
    } else if cause.ledger_cause().is_some() {
        NODE_SCENE_WORK_INTERRUPTION_CODE
    } else {
        NODE_SCENE_BUILD_INVARIANT_CODE
    }
}

fn prepare_node_post_evidence_failure(
    assembly: SceneStoreAssembly,
    evidence: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    plans: &NodeSceneMaterializationPlans,
    cause: StoreRuntimeError,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    match post_evidence_failure_code(&cause) {
        Some(cause) => prepare_node_rich_failure_from_assembly(
            assembly,
            evidence,
            plans.canonical_block_count,
            plans.node_count,
            cause,
        ),
        None => {
            prepare_node_minimal_fallback_from_assembly(assembly, node_scene_failure_code(&cause))
        }
    }
}

fn register_node_scene_canonical_commitment(
    assembly: &mut SceneStoreAssembly,
    keys: SceneStoreKeys,
    canonical_block_count: u32,
) -> Result<PendingNodeSceneCommitments, StoreRuntimeError> {
    assembly.try_commit_canonical(keys.dimension, u64::from(canonical_block_count))?;
    Ok(PendingNodeSceneCommitments)
}

fn prepare_validated_node_scene(
    candidate: ValidatedNodeSceneCandidate,
    reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    let ValidatedNodeSceneCandidate {
        store,
        refs,
        canonical_block_count,
        node_count,
        proof,
        pending_commitments,
    } = candidate;
    let generation = match reserve_generation() {
        Ok(generation) => generation,
        Err(SceneGenerationError::Exhausted) => {
            return prepare_node_rich_failure(
                store.downgrade_to_rich_failure(GENERATION_EXHAUSTED_CODE),
                canonical_block_count,
                node_count,
                GENERATION_EXHAUSTED_CODE,
            );
        }
    };
    let PendingNodeSceneCommitments = pending_commitments;
    match store.try_prepare(final_progress())? {
        PreparedStoreSuccessOutcome::Success(store) => Ok(PreparedNodeSceneConstruction::Success(
            PreparedNodeSceneSuccess {
                generation,
                store,
                refs,
                proof,
            },
        )),
        PreparedStoreSuccessOutcome::RichFailure(store) => {
            burn_generation_reservation(generation);
            Ok(PreparedNodeSceneConstruction::RichFailure(
                PreparedNodeSceneRichFailure { store },
            ))
        }
        PreparedStoreSuccessOutcome::MinimalFallback(store) => {
            burn_generation_reservation(generation);
            Ok(PreparedNodeSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
    }
}

fn prepare_node_rich_failure(
    failure: NodeSceneStoreFailureAssembly,
    canonical_block_count: u32,
    node_count: u32,
    cause: u32,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    let evidence = failure.evidence_root();
    finish_node_rich_failure_validation(
        failure.validate_rich_failure(evidence, |session, evidence| {
            validate_node_scene_evidence_graph(session, evidence, canonical_block_count, node_count)
        }),
        cause,
    )
}

fn prepare_node_rich_failure_from_assembly(
    assembly: SceneStoreAssembly,
    evidence: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    canonical_block_count: u32,
    node_count: u32,
    cause: u32,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    finish_node_rich_failure_validation(
        assembly.validate_rich_failure(evidence, |session, evidence| {
            validate_node_scene_evidence_graph(session, evidence, canonical_block_count, node_count)
        }),
        cause,
    )
}

fn finish_node_rich_failure_validation(
    validation: NodeSceneStoreFailureValidation,
    cause: u32,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    match validation {
        StoreFailureValidation::Valid(validated) => {
            map_node_prepared_failure(validated.try_prepare(cause, final_progress())?)
        }
        StoreFailureValidation::Rejected(rejected) => {
            let store = rejected.prepare_minimal_fallback_with(cause)?;
            Ok(PreparedNodeSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
        StoreFailureValidation::FallbackOnly(fallback) => {
            let store = fallback.prepare_minimal_fallback()?;
            Ok(PreparedNodeSceneConstruction::MinimalFallback(
                PreparedTracerSceneFallback { store },
            ))
        }
    }
}

fn map_node_prepared_failure(
    prepared: PreparedStoreFailureOutcome<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    Ok(match prepared {
        PreparedStoreFailureOutcome::RichFailure(store) => {
            PreparedNodeSceneConstruction::RichFailure(PreparedNodeSceneRichFailure { store })
        }
        PreparedStoreFailureOutcome::MinimalFallback(store) => {
            PreparedNodeSceneConstruction::MinimalFallback(PreparedTracerSceneFallback { store })
        }
    })
}

fn prepare_node_minimal_fallback_from_operation(
    operation: SceneStoreOperation,
    cause: u32,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    let store = operation.prepare_minimal_fallback(cause)?;
    Ok(PreparedNodeSceneConstruction::MinimalFallback(
        PreparedTracerSceneFallback { store },
    ))
}

fn prepare_node_minimal_fallback_from_assembly(
    assembly: SceneStoreAssembly,
    cause: u32,
) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
    let store = assembly.prepare_minimal_fallback_with(cause)?;
    Ok(PreparedNodeSceneConstruction::MinimalFallback(
        PreparedTracerSceneFallback { store },
    ))
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

impl ValidatedNodeScene {
    pub(in crate::canonical_scene::store) const fn generation(&self) -> SceneGenerationId {
        self.generation
    }

    pub(in crate::canonical_scene::store) fn account(&self) -> &PhysicalStorageAccount {
        self.canonical.account()
    }

    pub(in crate::canonical_scene::store) fn source_node_count(&self) -> Option<usize> {
        let directory = self.canonical.records(self.refs.directory.into_store())?;
        let canonical_count = directory.scalar::<u32>(0, node_scene_directory_field::NODE_COUNT)?;
        usize::try_from(canonical_count).ok()?.checked_sub(1)
    }

    pub(in crate::canonical_scene::store) fn source_root_count(&self) -> Option<usize> {
        let directory = self.canonical.records(self.refs.directory.into_store())?;
        usize::try_from(directory.scalar::<u32>(0, node_scene_directory_field::SOURCE_ROOT_COUNT)?)
            .ok()
    }

    pub(in crate::canonical_scene::store) fn canonical_block_count(&self) -> usize {
        self.canonical.len()
    }

    /// Returns one ordered child source index in canonical authored order.
    ///
    /// `parent_source_index == None` selects the synthetic root. A present parent
    /// index addresses one authored node and excludes the synthetic root.
    pub(in crate::canonical_scene::store) fn child(
        &self,
        parent_source_index: Option<usize>,
        child_ordinal: usize,
    ) -> Option<usize> {
        let parent_row = match parent_source_index {
            None => 0,
            Some(source_index) => source_index.checked_add(1)?,
        };
        let nodes = self.canonical.records(self.refs.nodes.into_store())?;
        if parent_row >= nodes.rows() {
            return None;
        }
        let reference_offset = usize::try_from(
            nodes.scalar::<u32>(parent_row, node_scene_node_field::CHILD_REFERENCE_OFFSET)?,
        )
        .ok()?;
        let child_count = usize::try_from(
            nodes.scalar::<u32>(parent_row, node_scene_node_field::CHILD_REFERENCE_COUNT)?,
        )
        .ok()?;
        if child_ordinal >= child_count {
            return None;
        }
        let reference_row = reference_offset.checked_add(child_ordinal)?;
        let references = self.canonical.records(self.refs.references?.into_store())?;
        usize::try_from(
            references.scalar::<u32>(reference_row, node_scene_child_reference_field::CHILD)?,
        )
        .ok()?
        .checked_sub(1)
    }

    /// Derives the parent source index from canonical ordered-child ranges.
    ///
    /// The outer option rejects an out-of-range source index. The inner option is
    /// `None` for a source root and excludes the synthetic canonical root.
    pub(in crate::canonical_scene::store) fn parent(
        &self,
        source_index: usize,
    ) -> Option<Option<usize>> {
        let row = source_index.checked_add(1)?;
        let nodes = self.canonical.records(self.refs.nodes.into_store())?;
        if row >= nodes.rows() {
            return None;
        }
        let references = self.canonical.records(self.refs.references?.into_store())?;
        if references.scalar::<u32>(source_index, node_scene_child_reference_field::CHILD)?
            != u32::try_from(row).ok()?
        {
            return None;
        }
        let mut lower = 0_usize;
        let mut upper = nodes.rows();
        while lower < upper {
            let parent_row = lower.checked_add((upper - lower) / 2)?;
            let reference_offset = usize::try_from(
                nodes.scalar::<u32>(parent_row, node_scene_node_field::CHILD_REFERENCE_OFFSET)?,
            )
            .ok()?;
            let child_count = usize::try_from(
                nodes.scalar::<u32>(parent_row, node_scene_node_field::CHILD_REFERENCE_COUNT)?,
            )
            .ok()?;
            if reference_offset.checked_add(child_count)? <= source_index {
                lower = parent_row.checked_add(1)?;
            } else {
                upper = parent_row;
            }
        }

        let parent_row = lower;
        let reference_offset = usize::try_from(
            nodes.scalar::<u32>(parent_row, node_scene_node_field::CHILD_REFERENCE_OFFSET)?,
        )
        .ok()?;
        let child_count = usize::try_from(
            nodes.scalar::<u32>(parent_row, node_scene_node_field::CHILD_REFERENCE_COUNT)?,
        )
        .ok()?;
        let reference_end = reference_offset.checked_add(child_count)?;
        (reference_offset..reference_end)
            .contains(&source_index)
            .then_some(if parent_row == 0 {
                None
            } else {
                Some(parent_row - 1)
            })
    }

    pub(in crate::canonical_scene::store) fn authored_transform(
        &self,
        source_index: usize,
    ) -> Option<AuthoredTransform> {
        let row = source_index.checked_add(1)?;
        let nodes = self.canonical.records(self.refs.nodes.into_store())?;
        let kind = nodes.scalar::<u8>(row, node_scene_node_field::TRANSFORM_KIND)?;
        let offset =
            usize::try_from(nodes.scalar::<u32>(row, node_scene_node_field::TRANSFORM_OFFSET)?)
                .ok()?;
        let length =
            usize::try_from(nodes.scalar::<u32>(row, node_scene_node_field::TRANSFORM_LENGTH)?)
                .ok()?;
        let transforms = self
            .canonical
            .sequence(self.refs.transforms?.into_store())?;
        let values = transforms
            .as_slice()
            .get(offset..offset.checked_add(length)?)?;
        match kind {
            NODE_SCENE_TRANSFORM_MATRIX => {
                let values = <&[f32; NODE_SCENE_MATRIX_SCALAR_COUNT]>::try_from(values).ok()?;
                AuthoredTransform::try_matrix(Mat4 {
                    cols: [
                        [values[0], values[1], values[2], values[3]],
                        [values[4], values[5], values[6], values[7]],
                        [values[8], values[9], values[10], values[11]],
                        [values[12], values[13], values[14], values[15]],
                    ],
                })
                .ok()
            }
            NODE_SCENE_TRANSFORM_TRS => {
                let values = <&[f32; NODE_SCENE_TRS_SCALAR_COUNT]>::try_from(values).ok()?;
                AuthoredTransform::try_trs(
                    Vec3::new(values[0], values[1], values[2]),
                    Quat::from_xyzw(values[3], values[4], values[5], values[6]),
                    Vec3::new(values[7], values[8], values[9]),
                )
                .ok()
            }
            _ => None,
        }
    }
}

pub(in crate::canonical_scene::store) struct NodeSceneSuccessReport {
    outcome_evidence: PublishedOutcomeEvidenceStore<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
    ledger_evidence: StoreLedgerEvidence,
}

impl NodeSceneSuccessReport {
    pub(in crate::canonical_scene::store) fn outcome_evidence_block_count(&self) -> usize {
        self.outcome_evidence.len()
    }

    pub(in crate::canonical_scene::store) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        &self.ledger_evidence
    }
}

pub(in crate::canonical_scene::store) struct NodeSceneSuccessParts {
    pub(in crate::canonical_scene::store) scene: ValidatedNodeScene,
    pub(in crate::canonical_scene::store) report: NodeSceneSuccessReport,
}

pub(in crate::canonical_scene::store) struct NodeSceneRichFailure {
    outcome_evidence: PublishedRichFailureStore<RecordBlock, NodeSceneOutcomeEvidenceSchema>,
    ledger_evidence: StoreLedgerEvidence,
}

impl NodeSceneRichFailure {
    pub(in crate::canonical_scene::store) fn account(&self) -> &PhysicalStorageAccount {
        self.outcome_evidence.account()
    }

    pub(in crate::canonical_scene::store) const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        &self.ledger_evidence
    }

    pub(in crate::canonical_scene::store) fn outcome_evidence_block_count(&self) -> usize {
        self.outcome_evidence.outcome_evidence().len()
    }
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

impl PreparedNodeSceneConstruction {
    pub(in crate::canonical_scene::store) fn handoff(self) -> NodeSceneConstructionTerminal {
        match self {
            Self::Success(prepared) => {
                let published = prepared.store.handoff();
                NodeSceneConstructionTerminal::Success(NodeSceneSuccessParts {
                    scene: ValidatedNodeScene {
                        generation: prepared.generation.into_id(),
                        canonical: published.canonical,
                        refs: prepared.refs,
                        _proof: prepared.proof,
                    },
                    report: NodeSceneSuccessReport {
                        outcome_evidence: published.outcome_evidence,
                        ledger_evidence: published.ledger_evidence,
                    },
                })
            }
            Self::RichFailure(prepared) => {
                let published = prepared.store.handoff();
                NodeSceneConstructionTerminal::RichFailure(NodeSceneRichFailure {
                    outcome_evidence: published.outcome_evidence,
                    ledger_evidence: published.ledger_evidence,
                })
            }
            Self::MinimalFallback(prepared) => {
                let published = prepared.store.handoff();
                NodeSceneConstructionTerminal::MinimalFallback(TracerSceneFallback {
                    fallback: published.fallback,
                    ledger_evidence: published.ledger_evidence,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use baozi_operation::{
        CancellationToken, PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
        PhysicalStorageAccountLimits, StorePreparationFailureClass, StoreTerminalCause,
    };

    use crate::SceneValidationLimits;
    use crate::canonical_scene::store::construction::{
        STORE_DIMENSION, SceneConstructionLimits, SceneConstructionOperation,
        SceneConstructionTerminal, TwoNodeAuthoredMatrices,
    };
    use crate::canonical_scene::store::production::{
        NodeAuthoringBatch, NodeAuthoringFailure, NodeAuthoringLimit, NodeAuthoringNode,
        NodeAuthoringSemanticError, NodeLocalId, NodeParent, Presence,
    };
    use crate::entity::AuthoredTransform;
    use crate::identity::{SceneGenerationError, SceneGenerationId};
    use crate::math::{Mat4, Quat, Vec3};

    use super::{
        NODE_SCENE_BUILD_INVARIANT_CODE, NODE_SCENE_STORAGE_REJECTION_CODE, NodeSceneBuildFaults,
        NodeSceneConstructionRejection, NodeSceneConstructionTerminal, NodeSceneRichFailure,
        PreparedNodeSceneConstruction, post_evidence_build_failure_code,
    };

    const SINGLE_TRS_WORK_BEFORE_FIRST_PAYLOAD_WRITE: u64 = 43;
    const SINGLE_TRS_SUCCESS_WORK: u64 = 78;
    const SINGLE_TRS_WORK_BEFORE_EVIDENCE_VALIDATION: u64 = 77;
    const SINGLE_TRS_ROOTS: [NodeLocalId; 1] = [NodeLocalId::new(0)];
    const SINGLE_TRS_NODES: [NodeAuthoringNode; 1] = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
    )];

    fn test_limits() -> SceneConstructionLimits {
        SceneConstructionLimits {
            operation_live_bytes: u64::MAX,
            dimension_live_bytes: u64::MAX,
            attempted_work: u64::MAX,
            consumed_work: u64::MAX,
            committed_output: u64::MAX,
            published_evidence: u64::MAX,
        }
    }

    fn test_factory() -> PhysicalStorageAccountFactory {
        PhysicalStorageAccountFactory::try_new(
            PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1)
                .expect("node Scene test factory limits"),
        )
        .expect("node Scene test factory")
    }

    fn test_limits_with_work(work: u64) -> SceneConstructionLimits {
        SceneConstructionLimits {
            attempted_work: work,
            consumed_work: work,
            ..test_limits()
        }
    }

    fn single_trs_batch() -> NodeAuthoringBatch<'static> {
        NodeAuthoringBatch::new(Presence::Present(&SINGLE_TRS_ROOTS), &SINGLE_TRS_NODES)
    }

    fn parent(node: u32, child_ordinal: u32) -> Presence<NodeParent> {
        Presence::Present(NodeParent::new(NodeLocalId::new(node), child_ordinal))
    }

    fn test_account(
        factory: &PhysicalStorageAccountFactory,
        max_live_allocations: u64,
    ) -> baozi_operation::PhysicalStorageAccount {
        factory
            .try_create_account(
                PhysicalStorageAccountLimits::new(u64::MAX, max_live_allocations)
                    .expect("node Scene test account limits"),
            )
            .expect("node Scene test account")
    }

    fn successful_parts(prepared: PreparedNodeSceneConstruction) -> super::NodeSceneSuccessParts {
        match prepared.handoff() {
            NodeSceneConstructionTerminal::Success(parts) => parts,
            NodeSceneConstructionTerminal::RichFailure(_) => {
                panic!("valid node Scene must not publish rich failure")
            }
            NodeSceneConstructionTerminal::MinimalFallback(_) => {
                panic!("valid node Scene must not publish minimal fallback")
            }
        }
    }

    fn rich_failure(prepared: PreparedNodeSceneConstruction) -> NodeSceneRichFailure {
        match prepared.handoff() {
            NodeSceneConstructionTerminal::Success(_) => {
                panic!("rejected node Scene must not publish canonical storage")
            }
            NodeSceneConstructionTerminal::RichFailure(failure) => failure,
            NodeSceneConstructionTerminal::MinimalFallback(_) => {
                panic!("recoverable node Scene rejection must preserve rich evidence")
            }
        }
    }

    fn assert_released(
        failure: NodeSceneRichFailure,
        account: &baozi_operation::PhysicalStorageAccount,
    ) {
        assert!(failure.account().is_same_account(account));
        assert_eq!(failure.outcome_evidence_block_count(), 1);
        drop(failure);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    fn terminal_failure_code(evidence: &baozi_operation::StoreLedgerEvidence) -> u32 {
        match evidence.header().terminal_cause() {
            StoreTerminalCause::Failure(cause) => cause.code(),
            StoreTerminalCause::Success => {
                panic!("rejected node Scene must freeze a failure cause")
            }
            _ => panic!("unknown terminal cause cannot prove a node Scene failure"),
        }
    }

    fn terminal_failure_class(
        evidence: &baozi_operation::StoreLedgerEvidence,
    ) -> StorePreparationFailureClass {
        match evidence.header().terminal_cause() {
            StoreTerminalCause::Failure(cause) => cause.class(),
            StoreTerminalCause::Success => {
                panic!("rejected node Scene must freeze a failure cause")
            }
            _ => panic!("unknown terminal cause cannot prove a node Scene failure"),
        }
    }

    fn matrix_transform(translation_x: f32) -> AuthoredTransform {
        AuthoredTransform::try_matrix(Mat4 {
            cols: [
                [1.0, -0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [translation_x, 2.0, -3.0, 1.0],
            ],
        })
        .expect("finite affine authored matrix")
    }

    fn trs_transform(translation_x: f32) -> AuthoredTransform {
        AuthoredTransform::try_trs(
            Vec3::new(translation_x, -0.0, 3.5),
            Quat::IDENTITY,
            Vec3::new(1.0, 2.0, 0.5),
        )
        .expect("valid authored TRS")
    }

    #[test]
    fn source_ordered_forest_materializes_canonical_graph_and_preserves_authored_bits() {
        let factory = test_factory();
        let account = test_account(&factory, 256);
        let generation_source = AtomicU64::new(0);
        let generation_before = SceneGenerationId::reserve_from(&generation_source)
            .expect("generation before node Scene construction")
            .into_id();
        let transforms = [
            matrix_transform(-4.0),
            trs_transform(1.0),
            trs_transform(2.0),
            matrix_transform(4.0),
            trs_transform(5.0),
        ];
        let nodes = [
            NodeAuthoringNode::new(parent(3, 1), transforms[0]),
            NodeAuthoringNode::new(Presence::Absent, transforms[1]),
            NodeAuthoringNode::new(parent(3, 0), transforms[2]),
            NodeAuthoringNode::new(Presence::Absent, transforms[3]),
            NodeAuthoringNode::new(parent(1, 0), transforms[4]),
        ];
        let roots = [NodeLocalId::new(3), NodeLocalId::new(1)];
        let prepared = SceneConstructionOperation::with_account(&account, test_limits())
            .expect("fresh node Scene operation")
            .try_validate_node_batch(
                NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
                SceneValidationLimits::default(),
            )
            .unwrap_or_else(|_| panic!("valid source-ordered forest must validate"))
            .prepare_with_generation_source(|| SceneGenerationId::reserve_from(&generation_source))
            .expect("valid source-ordered forest must prepare");
        let before_handoff = account.snapshot();
        let parts = successful_parts(prepared);
        let after_handoff = account.snapshot();
        let generation_after = SceneGenerationId::reserve_from(&generation_source)
            .expect("generation after node Scene construction")
            .into_id();

        assert_eq!(parts.scene.source_node_count(), Some(5));
        assert_eq!(parts.scene.source_root_count(), Some(2));
        assert_eq!(parts.scene.canonical_block_count(), 4);
        assert!(parts.scene.account().is_same_account(&account));
        assert!(generation_before < parts.scene.generation());
        assert!(parts.scene.generation() < generation_after);
        assert_eq!(generation_source.load(Ordering::Relaxed), 3);
        assert_eq!(before_handoff, after_handoff);
        assert_eq!(parts.scene.parent(0), Some(None));
        assert_eq!(parts.scene.parent(1), Some(None));
        assert_eq!(parts.scene.parent(2), Some(Some(0)));
        assert_eq!(parts.scene.parent(3), Some(Some(0)));
        assert_eq!(parts.scene.parent(4), Some(Some(1)));
        assert_eq!(parts.scene.parent(5), None);
        assert_eq!(parts.scene.child(None, 0), Some(0));
        assert_eq!(parts.scene.child(None, 1), Some(1));
        assert_eq!(parts.scene.child(None, 2), None);
        assert_eq!(parts.scene.child(Some(0), 0), Some(2));
        assert_eq!(parts.scene.child(Some(0), 1), Some(3));
        assert_eq!(parts.scene.child(Some(0), 2), None);
        assert_eq!(parts.scene.child(Some(1), 0), Some(4));
        assert_eq!(parts.scene.child(Some(2), 0), None);

        let canonical_local_order = [3, 1, 2, 0, 4];
        for (canonical_source, local) in canonical_local_order.into_iter().enumerate() {
            let inspected = parts
                .scene
                .authored_transform(canonical_source)
                .expect("canonical authored transform");
            assert!(inspected.has_same_authored_bits(transforms[local]));
        }
        assert_eq!(parts.report.outcome_evidence_block_count(), 1);
        let ledger = parts.report.ledger_evidence();
        let header = ledger.header();
        assert!(matches!(
            header.terminal_cause(),
            StoreTerminalCause::Success
        ));
        assert_eq!(header.dimension_count(), 1);
        assert_eq!(header.phase_count(), 1);
        assert_eq!(header.current_live_bytes(), 0);
        assert!(header.peak_live_bytes() > 0);
        assert_eq!(header.active_reservations(), 0);
        let dimension = ledger
            .dimension_at(0)
            .expect("node Scene dimension evidence");
        assert!(dimension.attempted() > 0);
        assert_eq!(dimension.consumed(), dimension.attempted());
        assert_eq!(dimension.current_live_bytes(), 0);
        assert!(dimension.peak_live_bytes() > 0);
        assert_eq!(dimension.pending_publication(), 0);
        assert_eq!(dimension.committed(), 4);
        assert_eq!(dimension.committed_canonical_output(), 4);
        assert_eq!(
            dimension.published_outcome_evidence(),
            2,
            "startup fallback and successful outcome evidence are both accounted"
        );

        drop(parts);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        drop(account);
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn present_empty_scene_publishes_only_directory_and_node_table() {
        let factory = test_factory();
        let account = test_account(&factory, 256);
        let roots = [];
        let nodes = [];
        let prepared = SceneConstructionOperation::with_account(&account, test_limits())
            .expect("empty node Scene operation")
            .try_validate_node_batch(
                NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
                SceneValidationLimits::default(),
            )
            .unwrap_or_else(|_| panic!("present empty node Scene must validate"))
            .prepare()
            .expect("present empty node Scene must prepare");
        let parts = successful_parts(prepared);

        assert_eq!(parts.scene.source_node_count(), Some(0));
        assert_eq!(parts.scene.source_root_count(), Some(0));
        assert_eq!(parts.scene.canonical_block_count(), 2);
        assert_eq!(parts.scene.parent(0), None);
        assert_eq!(parts.scene.child(None, 0), None);
        assert_eq!(parts.scene.authored_transform(0), None);

        drop(parts);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn canonical_logical_byte_rejection_never_opens_publication() {
        let factory = test_factory();
        let account = test_account(&factory, 256);
        let operation = SceneConstructionOperation::with_account(&account, test_limits())
            .expect("canonical-byte-bounded node Scene operation");
        let before = account.snapshot();

        let rejected = match operation.try_validate_node_batch(
            single_trs_batch(),
            SceneValidationLimits::default().with_max_canonical_bytes(133),
        ) {
            Ok(_) => panic!("one byte below the canonical requirement must reject"),
            Err(rejected) => rejected,
        };
        assert!(matches!(
            rejected.failure(),
            NodeSceneConstructionRejection::Authoring(NodeAuthoringFailure::Semantic(
                NodeAuthoringSemanticError::LimitExceeded {
                    limit: NodeAuthoringLimit::CanonicalBytes,
                    actual: 134,
                    maximum: 133,
                }
            ))
        ));
        let after_rejection = account.snapshot();
        assert_eq!(
            after_rejection, before,
            "authoring rejection must not enter physical publication"
        );

        let terminal = rejected
            .prepare_minimal_fallback()
            .expect("canonical byte rejection fallback")
            .handoff();
        assert!(matches!(
            &terminal,
            NodeSceneConstructionTerminal::MinimalFallback(_)
        ));
        drop(terminal);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn cancellation_after_authoring_validation_publishes_rich_failure() {
        let factory = test_factory();
        let account = test_account(&factory, 256);
        let cancellation = CancellationToken::new();
        let validated = SceneConstructionOperation::with_account_and_cancellation_for_test(
            &account,
            test_limits(),
            cancellation.clone(),
        )
        .expect("cancellable node Scene operation")
        .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
        .unwrap_or_else(|_| panic!("single-node input must validate before cancellation"));

        cancellation.cancel();
        let failure = rich_failure(
            validated
                .prepare()
                .expect("post-validation cancellation must prepare rich failure"),
        );
        let work = failure
            .ledger_evidence()
            .dimension_by_semantic(STORE_DIMENSION)
            .expect("node Scene work evidence");
        assert_eq!(work.attempted(), SINGLE_TRS_WORK_BEFORE_FIRST_PAYLOAD_WRITE);
        assert_eq!(work.consumed(), SINGLE_TRS_WORK_BEFORE_FIRST_PAYLOAD_WRITE);
        assert_released(failure, &account);
    }

    #[test]
    fn materialization_work_budget_interruption_publishes_rich_failure() {
        let factory = test_factory();
        let account = test_account(&factory, 256);
        let validated = SceneConstructionOperation::with_account(
            &account,
            test_limits_with_work(SINGLE_TRS_WORK_BEFORE_FIRST_PAYLOAD_WRITE),
        )
        .expect("work-bounded node Scene operation")
        .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
        .unwrap_or_else(|_| panic!("work budget must admit authoring validation exactly"));

        let failure = rich_failure(
            validated
                .prepare()
                .expect("materialization budget interruption must prepare rich failure"),
        );
        let work = failure
            .ledger_evidence()
            .dimension_by_semantic(STORE_DIMENSION)
            .expect("node Scene work evidence");
        assert_eq!(
            work.attempted(),
            SINGLE_TRS_WORK_BEFORE_FIRST_PAYLOAD_WRITE + 10
        );
        assert_eq!(work.consumed(), SINGLE_TRS_WORK_BEFORE_FIRST_PAYLOAD_WRITE);
        assert_released(failure, &account);
    }

    #[test]
    fn central_validation_work_budget_interruption_publishes_rich_failure() {
        const WORK_LIMIT: u64 = 61;

        let factory = test_factory();
        let account = test_account(&factory, 256);
        let validated =
            SceneConstructionOperation::with_account(&account, test_limits_with_work(WORK_LIMIT))
                .expect("validation-bounded node Scene operation")
                .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
                .unwrap_or_else(|_| panic!("work budget must admit authoring validation"));

        let failure = rich_failure(
            validated
                .prepare()
                .expect("central validation budget interruption must prepare rich failure"),
        );
        let work = failure
            .ledger_evidence()
            .dimension_by_semantic(STORE_DIMENSION)
            .expect("node Scene work evidence");
        assert_eq!(work.attempted(), WORK_LIMIT + 1);
        assert_eq!(work.consumed(), WORK_LIMIT);
        assert_released(failure, &account);
    }

    #[test]
    fn generation_exhaustion_discards_canonical_storage_and_preserves_evidence() {
        let factory = test_factory();
        let account = test_account(&factory, 256);
        let validated = SceneConstructionOperation::with_account(&account, test_limits())
            .expect("node Scene generation test operation")
            .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
            .unwrap_or_else(|_| panic!("single-node input must validate"));

        let failure = rich_failure(
            validated
                .prepare_with_generation_source(|| Err(SceneGenerationError::Exhausted))
                .expect("generation exhaustion must prepare rich failure"),
        );
        let work = failure
            .ledger_evidence()
            .dimension_by_semantic(STORE_DIMENSION)
            .expect("node Scene work evidence");
        assert_eq!(work.attempted(), SINGLE_TRS_SUCCESS_WORK);
        assert_released(failure, &account);
    }

    #[test]
    fn authored_transform_bit_fault_is_a_build_invariant_rich_failure() {
        let factory = test_factory();
        let account = test_account(&factory, 256);
        let validated = SceneConstructionOperation::with_account(&account, test_limits())
            .expect("node Scene semantic fault operation")
            .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
            .unwrap_or_else(|_| panic!("single-node input must validate"));

        let failure = rich_failure(
            validated
                .prepare_with_faults(NodeSceneBuildFaults::altered_authored_transform())
                .expect("authored transform mismatch must prepare rich failure"),
        );
        let work = failure
            .ledger_evidence()
            .dimension_by_semantic(STORE_DIMENSION)
            .expect("node Scene work evidence");
        assert_eq!(work.attempted(), SINGLE_TRS_WORK_BEFORE_EVIDENCE_VALIDATION);
        assert_eq!(
            terminal_failure_code(failure.ledger_evidence()),
            NODE_SCENE_BUILD_INVARIANT_CODE
        );
        assert_released(failure, &account);
    }

    #[test]
    fn evidence_publication_limit_rejection_fails_closed() {
        let factory = test_factory();
        let account = test_account(&factory, 256);
        let limits = SceneConstructionLimits {
            published_evidence: 2,
            ..test_limits()
        };
        let validated = SceneConstructionOperation::with_account(&account, limits)
            .expect("baseline evidence limit must admit operation startup")
            .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
            .unwrap_or_else(|_| panic!("single-node input must validate"));

        let terminal = validated
            .prepare_with_faults(NodeSceneBuildFaults::excess_outcome_evidence_commitment())
            .expect("evidence publication rejection must prepare fail closed")
            .handoff();
        let NodeSceneConstructionTerminal::MinimalFallback(fallback) = terminal else {
            panic!("unbound evidence must terminate as minimal fallback");
        };
        assert_eq!(
            terminal_failure_class(fallback.ledger_evidence()),
            StorePreparationFailureClass::Ledger
        );
        drop(fallback);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn canonical_publication_limit_rejection_preserves_evidence_and_burns_generation() {
        let factory = test_factory();
        let account = test_account(&factory, 256);
        let generation_source = AtomicU64::new(0);
        let limits = SceneConstructionLimits {
            committed_output: 0,
            ..test_limits()
        };
        let validated = SceneConstructionOperation::with_account(&account, limits)
            .expect("zero canonical commitment limit operation")
            .try_validate_node_batch(single_trs_batch(), SceneValidationLimits::default())
            .unwrap_or_else(|_| panic!("single-node input must validate"));

        let terminal = validated
            .prepare_with_generation_source(|| SceneGenerationId::reserve_from(&generation_source))
            .expect("canonical publication rejection must prepare fail closed")
            .handoff();
        let NodeSceneConstructionTerminal::RichFailure(failure) = terminal else {
            panic!("canonical rejection must preserve validated outcome evidence");
        };
        assert_eq!(
            terminal_failure_class(failure.ledger_evidence()),
            StorePreparationFailureClass::Ledger
        );
        assert_eq!(generation_source.load(Ordering::Relaxed), 1);
        assert_released(failure, &account);

        let _next_generation = SceneGenerationId::reserve_from(&generation_source)
            .expect("generation after burned gap must remain reservable");
        assert_eq!(generation_source.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn cycle_and_derived_lineage_can_only_terminate_as_fallback() {
        let factory = test_factory();
        let account = test_account(&factory, 256);
        let cycle_nodes = [
            NodeAuthoringNode::new(Presence::Absent, AuthoredTransform::identity_trs()),
            NodeAuthoringNode::new(parent(2, 0), AuthoredTransform::identity_trs()),
            NodeAuthoringNode::new(parent(1, 0), AuthoredTransform::identity_trs()),
        ];
        let cycle_roots = [NodeLocalId::new(0)];
        let rejected = match SceneConstructionOperation::with_account(&account, test_limits())
            .expect("cycle rejection operation")
            .try_validate_node_batch(
                NodeAuthoringBatch::new(Presence::Present(&cycle_roots), &cycle_nodes),
                SceneValidationLimits::default(),
            ) {
            Ok(_) => panic!("cycle must not reach validated construction"),
            Err(rejected) => rejected,
        };
        assert!(matches!(
            rejected.failure(),
            &NodeSceneConstructionRejection::Authoring(NodeAuthoringFailure::Semantic(
                NodeAuthoringSemanticError::Cycle { .. }
            ))
        ));
        let cycle_terminal = rejected
            .prepare_minimal_fallback()
            .expect("cycle rejection fallback")
            .handoff();
        assert!(matches!(
            cycle_terminal,
            NodeSceneConstructionTerminal::MinimalFallback(_)
        ));
        drop(cycle_terminal);

        let legacy = SceneConstructionOperation::with_account(&account, test_limits())
            .expect("legacy source operation")
            .prepare_fresh_two_node(TwoNodeAuthoredMatrices::new(Mat4::IDENTITY, Mat4::IDENTITY))
            .expect("legacy source preparation");
        let legacy_scene = match legacy.handoff() {
            SceneConstructionTerminal::Success(parts) => parts.scene,
            SceneConstructionTerminal::RichFailure(_) => {
                panic!("legacy source must succeed")
            }
            SceneConstructionTerminal::MinimalFallback(_) => {
                panic!("legacy source must succeed")
            }
        };
        let empty_roots = [];
        let empty_nodes = [];
        let rejected = match SceneConstructionOperation::derived_from_exact_input(
            &legacy_scene,
            test_limits(),
        )
        .expect("derived node Scene operation")
        .try_validate_node_batch(
            NodeAuthoringBatch::new(Presence::Present(&empty_roots), &empty_nodes),
            SceneValidationLimits::default(),
        ) {
            Ok(_) => panic!("derived lineage must not reach validated construction"),
            Err(rejected) => rejected,
        };
        assert!(matches!(
            rejected.failure(),
            &NodeSceneConstructionRejection::DerivedLineage
        ));
        let derived_terminal = rejected
            .prepare_minimal_fallback()
            .expect("derived rejection fallback")
            .handoff();
        assert!(matches!(
            derived_terminal,
            NodeSceneConstructionTerminal::MinimalFallback(_)
        ));
        drop(derived_terminal);
        drop(legacy_scene);

        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn physical_budget_boundaries_release_storage_and_preserve_committed_evidence() {
        let factory = test_factory();
        let mut observed_fail_closed_publication = false;
        let mut observed_rich_storage_rejection = false;

        for max_live_allocations in 1..=128 {
            let account = test_account(&factory, max_live_allocations);
            let operation = match SceneConstructionOperation::with_account(&account, test_limits())
            {
                Ok(operation) => operation,
                Err(_) => {
                    assert_eq!(account.snapshot().live_retained_bytes(), 0);
                    assert_eq!(account.snapshot().live_retained_allocations(), 0);
                    drop(account);
                    continue;
                }
            };
            let roots = [NodeLocalId::new(0)];
            let nodes = [NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::identity_trs(),
            )];
            let validated = operation
                .try_validate_node_batch(
                    NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
                    SceneValidationLimits::default(),
                )
                .unwrap_or_else(|_| panic!("physical output limits must not reject semantics"));
            let prepared = validated
                .prepare()
                .expect("physical output failure must retain terminal fallback");
            let terminal = prepared.handoff();
            match terminal {
                NodeSceneConstructionTerminal::Success(parts) => drop(parts),
                NodeSceneConstructionTerminal::RichFailure(failure) => {
                    if terminal_failure_code(failure.ledger_evidence())
                        == NODE_SCENE_STORAGE_REJECTION_CODE
                    {
                        assert_eq!(failure.outcome_evidence_block_count(), 1);
                        observed_rich_storage_rejection = true;
                    }
                    drop(failure);
                }
                NodeSceneConstructionTerminal::MinimalFallback(fallback) => {
                    observed_fail_closed_publication = true;
                    drop(fallback);
                }
            }
            assert_eq!(account.snapshot().live_retained_bytes(), 0);
            assert_eq!(account.snapshot().live_retained_allocations(), 0);
            drop(account);

            if observed_fail_closed_publication && observed_rich_storage_rejection {
                break;
            }
        }

        assert!(
            observed_fail_closed_publication,
            "at least one low physical allocation budget must fail closed during publication"
        );
        assert!(
            observed_rich_storage_rejection,
            "a budget that admits evidence but rejects canonical storage must retain rich evidence"
        );
        assert_eq!(factory.snapshot().live_accounts(), 0);
    }

    #[test]
    fn internal_block_allocation_failure_is_a_storage_rejection() {
        assert_eq!(
            post_evidence_build_failure_code(
                baozi_operation::StoreBuildFailureClass::AllocationFailed,
            ),
            Some(NODE_SCENE_STORAGE_REJECTION_CODE)
        );
        assert_eq!(
            post_evidence_build_failure_code(
                baozi_operation::StoreBuildFailureClass::CapacityEnvelopeViolated,
            ),
            None
        );
    }
}
