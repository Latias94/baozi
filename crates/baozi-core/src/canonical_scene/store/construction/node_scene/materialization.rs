use baozi_operation::{
    OutcomeEvidenceRecordRef, RecordPlan, RetainedStoreBatchLimits, RetainedStoreBlockTablePlan,
    RetainedStorePublicationPlan, RetainedStoreReservation, RetainedStoreWriterPlan, SequencePlan,
    StoreBuildError, StoreControlledError, StorePlanError, StoreRuntimeError,
};

use crate::SceneValidationLimits;
use crate::canonical_scene::store::production::{
    NodeAuthoringBatch, OperationAuthoringControl, Presence, ValidatedNodeBatch,
    validate_node_batch,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, NODE_SCENE_CHILD_REFERENCE_FIELD_COUNT, NODE_SCENE_DIRECTORY_FIELD_COUNT,
    NODE_SCENE_EVIDENCE_FIELD_COUNT, NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE,
    NODE_SCENE_MATRIX_SCALAR_COUNT, NODE_SCENE_NODE_FIELD_COUNT, NODE_SCENE_SCHEMA_REVISION,
    NODE_SCENE_TRANSFORM_MATRIX, NODE_SCENE_TRANSFORM_SYNTHETIC, NODE_SCENE_TRANSFORM_TRS,
    NODE_SCENE_TRS_SCALAR_COUNT, NodeSceneAuthoredTransformSchema,
    NodeSceneChildReferenceTableSchema, NodeSceneDirectorySchema, NodeSceneNodeTableSchema,
    NodeSceneOutcomeEvidenceSchema, PRESENCE_ABSENT, PRESENCE_PRESENT,
};
use crate::entity::AuthoredTransformKind;
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};

use super::super::{
    FALLBACK_CODE, PreparedTracerSceneFallback, SceneConstructionError, SceneConstructionLineage,
    SceneConstructionOperation, SceneConstructionOwner, SceneStoreAssembly, SceneStoreKeys,
};
use super::publication::{
    node_scene_failure_code, post_evidence_failure_code,
    prepare_node_minimal_fallback_from_assembly, prepare_node_minimal_fallback_from_operation,
    prepare_node_rich_failure_from_assembly, prepare_node_scene_candidate,
};
use super::{
    FreshNodeSceneOwner, NODE_SCENE_BUILD_INVARIANT_CODE, NODE_SCENE_COMMITMENT_REJECTION_CODE,
    NodeSceneBlockRefs, NodeSceneBuildFaults, NodeSceneCandidate, NodeSceneCandidateOwner,
    NodeSceneConstructionRejection, NodeSceneDraft, NodeSceneWorkMeter,
    PreparedNodeSceneConstruction, RejectedNodeSceneConstruction, ValidatedNodeSceneConstruction,
};

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
struct NodeSceneDirectoryRefs {
    nodes: CanonicalBlockRef<NodeSceneNodeTableSchema>,
    references: Option<CanonicalBlockRef<NodeSceneChildReferenceTableSchema>>,
    transforms: Option<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>>,
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

    pub(super) fn prepare_with_generation_source(
        self,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        self.prepare_with_generation_source_and_faults(
            reserve_generation,
            NodeSceneBuildFaults::none(),
        )
    }

    #[cfg(test)]
    pub(super) fn prepare_with_faults(
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
