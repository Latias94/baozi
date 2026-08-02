use baozi_operation::{
    OutcomeEvidenceRecordRef, RecordPlan, RetainedStoreBatchLimits, RetainedStoreBlockTablePlan,
    RetainedStorePublicationPlan, RetainedStoreReservation, RetainedStoreWriterPlan, SequencePlan,
    StoreBuildError, StoreControlledError, StorePlanError, StoreRuntimeError,
};

use crate::SceneValidationLimits;
#[cfg(test)]
use crate::canonical_scene::store::production::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, DeformationAuthoringBatch,
    GeometryAuthoringBatch, NodeAuthoringBatch,
};
use crate::canonical_scene::store::production::{
    AnimationTargetCatalog, OperationAuthoringControl, Presence, SceneAuthoringBatch,
    ValidatedAnimationBatch, ValidatedAppearanceBatch, ValidatedDeformationBatch,
    ValidatedGeometryBatch, ValidatedNodeBatch, animation_limits_after_deformation,
    appearance_limits_after_nodes, deformation_limits_after_geometry,
    geometry_limits_after_appearance, validate_animation_batch, validate_appearance_batch,
    validate_deformation_batch, validate_geometry_batch, validate_node_batch,
    validate_scene_aggregate,
};
use crate::canonical_scene::store::schema::animation::AnimationDirectorySchema;
use crate::canonical_scene::store::schema::appearance::AppearanceDirectorySchema;
use crate::canonical_scene::store::schema::deformation::DeformationDirectorySchema;
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, NODE_SCENE_CHILD_REFERENCE_FIELD_COUNT, NODE_SCENE_EVIDENCE_FIELD_COUNT,
    NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE, NODE_SCENE_MATRIX_SCALAR_COUNT,
    NODE_SCENE_NODE_FIELD_COUNT, NODE_SCENE_TRANSFORM_MATRIX, NODE_SCENE_TRANSFORM_SYNTHETIC,
    NODE_SCENE_TRANSFORM_TRS, NODE_SCENE_TRS_SCALAR_COUNT, NodeSceneAuthoredTransformSchema,
    NodeSceneChildReferenceTableSchema, NodeSceneNodeTableSchema, NodeSceneOutcomeEvidenceSchema,
    PRESENCE_ABSENT, PRESENCE_PRESENT, SCENE_DIRECTORY_FIELD_COUNT, SCENE_SCHEMA_REVISION,
    SceneDirectorySchema, SceneGeometryDirectorySchema,
};
use crate::entity::AuthoredTransformKind;
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};

use super::super::{
    SceneAuthoringOperation, SceneConstructionError, SceneConstructionOperation,
    SceneConstructionOwner, SceneStoreAssembly, SceneStoreKeys, SceneStoreOperation,
};
use super::animation::{AnimationMaterializationPlans, AnimationTargetTableRefs, retain_animation};
use super::appearance::{AppearanceMaterializationPlans, retain_appearance};
use super::deformation::{
    DeformationDependencyTableRefs, DeformationMaterializationPlans, retain_deformation,
};
use super::geometry::{
    GeometryMaterializationPlans, GeometryReferenceScratch, GeometryReferenceScratchPlan,
    retain_geometry,
};
use super::publication::{
    node_scene_failure_code, post_evidence_failure_code,
    prepare_node_minimal_fallback_from_assembly, prepare_node_minimal_fallback_from_open_recovery,
    prepare_node_minimal_fallback_from_operation, prepare_node_rich_failure_from_assembly,
};
use super::{
    FreshNodeSceneOwner, NODE_SCENE_BUILD_INVARIANT_CODE, NODE_SCENE_COMMITMENT_REJECTION_CODE,
    NodeSceneBuildFaults, NodeSceneConstructionRejection, NodeSceneDraft, NodeSceneDraftBuild,
    NodeSceneWorkMeter, PendingNodeSceneDraft, PreparedNodeSceneConstruction,
    PreparedNodeSceneFailure, RejectedNodeSceneConstruction, SceneEvidenceCounts,
    ValidatedNodeSceneConstruction,
};

impl RejectedNodeSceneConstruction {
    pub(in crate::canonical_scene::store) const fn failure(
        &self,
    ) -> &NodeSceneConstructionRejection {
        &self.failure
    }

    pub(in crate::canonical_scene::store) fn prepare_rich_failure(
        self,
    ) -> Result<
        (
            PreparedNodeSceneConstruction,
            NodeSceneConstructionRejection,
        ),
        SceneConstructionError,
    > {
        let Self {
            operation:
                SceneConstructionOperation {
                    raw,
                    brand: _,
                    keys,
                },
            failure,
        } = self;
        let cause = failure.stable_code();
        let prepared = prepare_node_authoring_rejection(raw, keys, cause)?.into();
        Ok((prepared, failure))
    }
}

impl SceneConstructionOperation {
    #[cfg(test)]
    pub(super) fn try_validate_node_batch<'input>(
        self,
        input: NodeAuthoringBatch<'input>,
        limits: SceneValidationLimits,
    ) -> Result<ValidatedNodeSceneConstruction<'input>, RejectedNodeSceneConstruction> {
        self.try_validate_scene_batch(
            SceneAuthoringBatch::new(
                input,
                AppearanceAuthoringBatch::empty(),
                GeometryAuthoringBatch::empty(),
                DeformationAuthoringBatch::empty(),
                AnimationAuthoringBatch::empty(),
            ),
            limits,
        )
    }

    fn try_validate_scene_batch<'input>(
        mut self,
        input: SceneAuthoringBatch<'input>,
        limits: SceneValidationLimits,
    ) -> Result<ValidatedNodeSceneConstruction<'input>, RejectedNodeSceneConstruction> {
        let keys = self.keys;
        let validation = {
            let checkpoint = self.raw.control_checkpoint();
            let mut control =
                OperationAuthoringControl::new(checkpoint, keys.phase, keys.dimension);
            validate_node_batch(input.nodes(), limits, &mut control)
        };
        let nodes = match validation {
            Ok(nodes) => nodes,
            Err(failure) => {
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::NodeAuthoring(failure),
                });
            }
        };
        let appearance_limits = match appearance_limits_after_nodes(&nodes, limits) {
            Ok(appearance_limits) => appearance_limits,
            Err(failure) => {
                drop(nodes);
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::Aggregate(failure),
                });
            }
        };
        let appearance_validation = {
            let checkpoint = self.raw.control_checkpoint();
            let mut control =
                OperationAuthoringControl::new(checkpoint, keys.phase, keys.dimension);
            validate_appearance_batch(input.appearance(), appearance_limits, &mut control)
        };
        let appearance = match appearance_validation {
            Ok(appearance) => appearance,
            Err(failure) => {
                drop(nodes);
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::AppearanceAuthoring(failure),
                });
            }
        };
        let geometry_limits = match geometry_limits_after_appearance(&appearance, appearance_limits)
        {
            Ok(geometry_limits) => geometry_limits,
            Err(failure) => {
                drop(appearance);
                drop(nodes);
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::Aggregate(failure),
                });
            }
        };
        let geometry_validation = {
            let checkpoint = self.raw.control_checkpoint();
            let mut control =
                OperationAuthoringControl::new(checkpoint, keys.phase, keys.dimension);
            validate_geometry_batch(
                input.geometry(),
                input.nodes().nodes().len(),
                appearance.material_count(),
                geometry_limits,
                &mut control,
            )
        };
        let geometry = match geometry_validation {
            Ok(geometry) => geometry,
            Err(failure) => {
                drop(appearance);
                drop(nodes);
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::GeometryAuthoring(failure),
                });
            }
        };
        let deformation_limits = match deformation_limits_after_geometry(&geometry, geometry_limits)
        {
            Ok(deformation_limits) => deformation_limits,
            Err(failure) => {
                drop(geometry);
                drop(appearance);
                drop(nodes);
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::Aggregate(failure),
                });
            }
        };
        let deformation_validation = {
            let checkpoint = self.raw.control_checkpoint();
            let mut control =
                OperationAuthoringControl::new(checkpoint, keys.phase, keys.dimension);
            validate_deformation_batch(
                input.deformation(),
                &nodes,
                &geometry,
                deformation_limits,
                &mut control,
            )
        };
        let deformation = match deformation_validation {
            Ok(deformation) => deformation,
            Err(failure) => {
                drop(geometry);
                drop(appearance);
                drop(nodes);
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::DeformationAuthoring(failure),
                });
            }
        };
        let animation_limits =
            match animation_limits_after_deformation(&deformation, deformation_limits) {
                Ok(animation_limits) => animation_limits,
                Err(failure) => {
                    drop(deformation);
                    drop(geometry);
                    drop(appearance);
                    drop(nodes);
                    return Err(RejectedNodeSceneConstruction {
                        operation: self,
                        failure: NodeSceneConstructionRejection::Aggregate(failure),
                    });
                }
            };
        let animation_validation = {
            let checkpoint = self.raw.control_checkpoint();
            let mut control =
                OperationAuthoringControl::new(checkpoint, keys.phase, keys.dimension);
            let targets = AnimationTargetCatalog::new(&nodes, &deformation);
            validate_animation_batch(input.animations(), &targets, animation_limits, &mut control)
        };
        let animations = match animation_validation {
            Ok(animations) => animations,
            Err(failure) => {
                drop(deformation);
                drop(geometry);
                drop(appearance);
                drop(nodes);
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::AnimationAuthoring(failure),
                });
            }
        };
        if let Err(failure) = validate_scene_aggregate(
            &nodes,
            &appearance,
            &geometry,
            &deformation,
            &animations,
            limits,
        ) {
            drop(animations);
            drop(deformation);
            drop(geometry);
            drop(appearance);
            drop(nodes);
            return Err(RejectedNodeSceneConstruction {
                operation: self,
                failure: NodeSceneConstructionRejection::Aggregate(failure),
            });
        }
        let SceneConstructionOperation { raw, brand, keys } = self;
        Ok(ValidatedNodeSceneConstruction {
            animations,
            deformation,
            geometry,
            appearance,
            nodes,
            owner: FreshNodeSceneOwner { raw, brand, keys },
        })
    }
}

impl<'input> SceneAuthoringOperation<'input> {
    pub(in crate::canonical_scene::store) fn try_validate(
        self,
    ) -> Result<ValidatedNodeSceneConstruction<'input>, RejectedNodeSceneConstruction> {
        let Self {
            operation,
            input,
            validation_limits,
        } = self;
        operation.try_validate_scene_batch(input, validation_limits)
    }
}

#[derive(Clone, Copy)]
struct NodeSceneDirectoryRefs {
    nodes: CanonicalBlockRef<NodeSceneNodeTableSchema>,
    references: Option<CanonicalBlockRef<NodeSceneChildReferenceTableSchema>>,
    transforms: Option<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>>,
}

#[derive(Clone, Copy)]
struct SceneDomainDirectoryRefs {
    appearance: CanonicalBlockRef<AppearanceDirectorySchema>,
    geometry: CanonicalBlockRef<SceneGeometryDirectorySchema>,
    deformation: CanonicalBlockRef<DeformationDirectorySchema>,
    animation: CanonicalBlockRef<AnimationDirectorySchema>,
}

struct ValidatedNodeSceneBatches<'input> {
    animations: ValidatedAnimationBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
    deformation:
        ValidatedDeformationBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
    geometry: ValidatedGeometryBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
    appearance: ValidatedAppearanceBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
    nodes: ValidatedNodeBatch<'input, RetainedStoreReservation<SceneConstructionOwner>>,
}

struct NodeSceneMaterializationPlans {
    evidence: RecordPlan,
    transforms: Option<SequencePlan<f32>>,
    references: Option<RecordPlan>,
    nodes: RecordPlan,
    appearance: AppearanceMaterializationPlans,
    geometry: GeometryMaterializationPlans,
    deformation: DeformationMaterializationPlans,
    animation: AnimationMaterializationPlans,
    directory: RecordPlan,
    publication: RetainedStorePublicationPlan,
    canonical_block_count: u32,
    node_count: u32,
    source_node_count: u32,
    source_root_count: u32,
    transform_scalar_count: u32,
    payload_block_count: u32,
    build_invariant: StoreBuildError,
}

impl NodeSceneMaterializationPlans {
    fn evidence_counts(&self) -> SceneEvidenceCounts {
        SceneEvidenceCounts {
            canonical_block_count: self.canonical_block_count,
            node_count: self.node_count,
            image_count: self.appearance.image_count(),
            sampler_count: self.appearance.sampler_count(),
            texture_count: self.appearance.texture_count(),
            material_count: self.appearance.material_count(),
            material_binding_count: self.appearance.binding_count(),
            logical_mesh_count: self.geometry.logical_mesh_count(),
            primitive_count: self.geometry.primitive_count(),
            attribute_count: self.geometry.attribute_count(),
            mesh_instance_count: self.geometry.mesh_instance_count(),
            skin_count: self.deformation.skin_count(),
            logical_morph_count: self.deformation.logical_morph_count(),
            morph_definition_count: self.deformation.morph_definition_count(),
            primitive_deformation_count: self.deformation.primitive_count(),
            morph_target_count: self.deformation.morph_target_count(),
            morph_delta_count: self.deformation.morph_delta_count(),
            animation_clip_count: self.animation.clip_count(),
            animation_channel_count: self.animation.channel_count(),
            payload_block_count: self.payload_block_count,
        }
    }

    fn new(
        nodes_batch: &ValidatedNodeBatch<'_, RetainedStoreReservation<SceneConstructionOwner>>,
        appearance_batch: &ValidatedAppearanceBatch<
            '_,
            RetainedStoreReservation<SceneConstructionOwner>,
        >,
        geometry_batch: &ValidatedGeometryBatch<
            '_,
            RetainedStoreReservation<SceneConstructionOwner>,
        >,
        deformation_batch: &ValidatedDeformationBatch<
            '_,
            RetainedStoreReservation<SceneConstructionOwner>,
        >,
        animation_batch: &ValidatedAnimationBatch<
            '_,
            RetainedStoreReservation<SceneConstructionOwner>,
        >,
        keys: SceneStoreKeys,
    ) -> Result<Self, StorePlanError> {
        let node_count = u32::try_from(nodes_batch.canonical_node_count())
            .map_err(|_| StorePlanError::SizeOverflow)?;
        let source_node_count = node_count
            .checked_sub(1)
            .ok_or(StorePlanError::SizeOverflow)?;
        let source_node_count_usize =
            usize::try_from(source_node_count).map_err(|_| StorePlanError::SizeOverflow)?;
        let source_root_count = match nodes_batch.input().roots() {
            Presence::Present(roots) => {
                u32::try_from(roots.len()).map_err(|_| StorePlanError::SizeOverflow)?
            }
            Presence::Absent => return Err(StorePlanError::InvalidBlockTable),
        };
        let transform_scalar_count_u64 = nodes_batch
            .matrix_scalar_count()
            .checked_add(nodes_batch.trs_scalar_count())
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
        let appearance = AppearanceMaterializationPlans::new(appearance_batch)?;
        let geometry = GeometryMaterializationPlans::new(geometry_batch)?;
        let deformation = DeformationMaterializationPlans::new(deformation_batch)?;
        let animation = AnimationMaterializationPlans::new(animation_batch)?;
        let directory = RecordPlan::new(1, SCENE_DIRECTORY_FIELD_COUNT)?;
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
            .and_then(|count| count.checked_add(appearance.canonical_block_count()))
            .and_then(|count| count.checked_add(geometry.canonical_block_count()))
            .and_then(|count| count.checked_add(deformation.canonical_block_count()))
            .and_then(|count| count.checked_add(animation.canonical_block_count()))
            .ok_or(StorePlanError::SizeOverflow)?;
        let canonical_block_count =
            u32::try_from(canonical_block_count_usize).map_err(|_| StorePlanError::SizeOverflow)?;
        let payload_block_count = appearance
            .payload_block_count()
            .checked_add(geometry.payload_block_count())
            .and_then(|count| count.checked_add(deformation.payload_block_count()))
            .and_then(|count| count.checked_add(animation.payload_block_count()))
            .ok_or(StorePlanError::SizeOverflow)?;
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
            appearance,
            geometry,
            deformation,
            animation,
            directory,
            publication,
            canonical_block_count,
            node_count,
            source_node_count,
            source_root_count,
            transform_scalar_count,
            payload_block_count,
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

fn prepare_node_authoring_rejection(
    raw: SceneStoreOperation,
    keys: SceneStoreKeys,
    cause: u32,
) -> Result<PreparedNodeSceneFailure, SceneConstructionError> {
    let evidence_plan = match RecordPlan::new(1, NODE_SCENE_EVIDENCE_FIELD_COUNT) {
        Ok(plan) => plan,
        Err(_) => {
            return prepare_node_minimal_fallback_from_operation(
                raw,
                NODE_SCENE_BUILD_INVARIANT_CODE,
            );
        }
    };
    let evidence_table = match node_scene_writer_plan(keys, 2)
        .and_then(|writer| RetainedStoreBlockTablePlan::new(writer, 1, 1))
    {
        Ok(table) => table,
        Err(_) => {
            return prepare_node_minimal_fallback_from_operation(
                raw,
                NODE_SCENE_BUILD_INVARIANT_CODE,
            );
        }
    };
    let mut assembly = match raw
        .try_publication_assembly(RetainedStorePublicationPlan::rich_failure(evidence_table))
    {
        Ok(assembly) => assembly,
        Err(error) => {
            return prepare_node_minimal_fallback_from_open_recovery(error.into_disposition()?);
        }
    };
    let counts = SceneEvidenceCounts::authoring_rejection();
    let evidence = match retain_node_scene_evidence_counts(&mut assembly, evidence_plan, counts) {
        Ok(evidence) => evidence,
        Err(build_cause) => {
            return prepare_node_minimal_fallback_from_assembly(
                assembly,
                node_scene_failure_code(&build_cause),
            );
        }
    };
    if assembly
        .try_commit_outcome_evidence(keys.dimension, 1)
        .is_err()
    {
        return prepare_node_minimal_fallback_from_assembly(
            assembly,
            NODE_SCENE_COMMITMENT_REJECTION_CODE,
        );
    }
    prepare_node_rich_failure_from_assembly(assembly, evidence, counts, cause)
}

impl NodeSceneDraftBuild {
    fn failure(
        prepared: Result<PreparedNodeSceneFailure, SceneConstructionError>,
    ) -> Result<Self, SceneConstructionError> {
        prepared.map(Self::Failure)
    }

    pub(super) fn prepare_with_generation_source(
        self,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        match self {
            Self::Draft { draft, owner } => {
                owner.prepare_with_generation_source(draft, reserve_generation)
            }
            Self::Failure(failure) => Ok(failure.into()),
        }
    }
}

impl ValidatedNodeSceneConstruction<'_> {
    /// Materializes and centrally validates only the batch already coupled to this typestate.
    pub(in crate::canonical_scene::store) fn prepare(
        self,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        self.prepare_with_generation_source(SceneGenerationId::reserve)
    }

    pub(super) fn prepare_with_generation_source(
        self,
        reserve_generation: impl FnOnce() -> Result<ReservedSceneGeneration, SceneGenerationError>,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        self.finish_draft_with_faults(NodeSceneBuildFaults::none())?
            .prepare_with_generation_source(reserve_generation)
    }

    #[cfg(test)]
    pub(super) fn prepare_with_faults(
        self,
        faults: NodeSceneBuildFaults,
    ) -> Result<PreparedNodeSceneConstruction, SceneConstructionError> {
        self.finish_draft_with_faults(faults)?
            .prepare_with_generation_source(SceneGenerationId::reserve)
    }

    /// Freezes an owned draft after materialization and releases every source-backed batch.
    pub(in crate::canonical_scene::store) fn finish_draft(
        self,
    ) -> Result<NodeSceneDraftBuild, SceneConstructionError> {
        self.finish_draft_with_faults(NodeSceneBuildFaults::none())
    }

    fn finish_draft_with_faults(
        self,
        mut faults: NodeSceneBuildFaults,
    ) -> Result<NodeSceneDraftBuild, SceneConstructionError> {
        let Self {
            animations,
            deformation,
            geometry: geometry_batch,
            appearance,
            nodes: node_batch,
            owner,
        } = self;
        let batches = ValidatedNodeSceneBatches {
            animations,
            deformation,
            geometry: geometry_batch,
            appearance,
            nodes: node_batch,
        };
        let FreshNodeSceneOwner { raw, brand, keys } = owner;
        let plans = match NodeSceneMaterializationPlans::new(
            &batches.nodes,
            &batches.appearance,
            &batches.geometry,
            &batches.deformation,
            &batches.animations,
            keys,
        ) {
            Ok(plans) => plans,
            Err(_cause) => {
                drop(batches);
                return NodeSceneDraftBuild::failure(prepare_node_minimal_fallback_from_operation(
                    raw,
                    NODE_SCENE_BUILD_INVARIANT_CODE,
                ));
            }
        };
        let mut assembly: SceneStoreAssembly = match raw.try_publication_assembly(plans.publication)
        {
            Ok(assembly) => assembly,
            Err(error) => {
                drop(batches);
                return NodeSceneDraftBuild::failure(
                    prepare_node_minimal_fallback_from_open_recovery(error.into_disposition()?),
                );
            }
        };

        let evidence = match retain_node_scene_evidence(&mut assembly, &plans) {
            Ok(evidence) => evidence,
            Err(cause) => {
                drop(batches);
                return NodeSceneDraftBuild::failure(prepare_node_minimal_fallback_from_assembly(
                    assembly,
                    node_scene_failure_code(&cause),
                ));
            }
        };
        if let Err(_cause) =
            assembly.try_commit_outcome_evidence(keys.dimension, faults.outcome_evidence_commitment)
        {
            drop(batches);
            return NodeSceneDraftBuild::failure(prepare_node_minimal_fallback_from_assembly(
                assembly,
                NODE_SCENE_COMMITMENT_REJECTION_CODE,
            ));
        }
        let geometry_scratch_plan =
            match try_plan_geometry_reference_scratch(&mut assembly, &batches.geometry) {
                Ok(plan) => plan,
                Err(cause) => {
                    drop(batches);
                    return NodeSceneDraftBuild::failure(prepare_node_post_evidence_failure(
                        assembly, evidence, &plans, cause,
                    ));
                }
            };
        let mut work = NodeSceneWorkMeter::new();
        let transforms = match plans.transforms {
            Some(plan) => match retain_node_scene_transforms(
                &mut assembly,
                plan,
                &batches.nodes,
                &plans,
                keys,
                &mut work,
                &mut faults,
            ) {
                Ok(transforms) => Some(transforms),
                Err(cause) => {
                    drop(batches);
                    return NodeSceneDraftBuild::failure(prepare_node_post_evidence_failure(
                        assembly, evidence, &plans, cause,
                    ));
                }
            },
            None => None,
        };
        let references = match plans.references {
            Some(plan) => match retain_node_scene_references(
                &mut assembly,
                plan,
                &batches.nodes,
                &plans,
                keys,
                &mut work,
            ) {
                Ok(references) => Some(references),
                Err(cause) => {
                    drop(batches);
                    return NodeSceneDraftBuild::failure(prepare_node_post_evidence_failure(
                        assembly, evidence, &plans, cause,
                    ));
                }
            },
            None => None,
        };
        let nodes_ref = match retain_node_scene_nodes(
            &mut assembly,
            plans.nodes,
            transforms,
            &batches.nodes,
            &plans,
            keys,
            &mut work,
        ) {
            Ok(nodes) => nodes,
            Err(cause) => {
                drop(batches);
                return NodeSceneDraftBuild::failure(prepare_node_post_evidence_failure(
                    assembly, evidence, &plans, cause,
                ));
            }
        };
        let appearance_refs = match retain_appearance(
            &mut assembly,
            &plans.appearance,
            &batches.appearance,
            keys,
            &mut work,
            plans.build_invariant,
        ) {
            Ok(appearance) => appearance,
            Err(cause) => {
                drop(batches);
                return NodeSceneDraftBuild::failure(prepare_node_post_evidence_failure(
                    assembly, evidence, &plans, cause,
                ));
            }
        };
        let geometry_scratch =
            match try_create_geometry_reference_scratch(&mut assembly, keys, geometry_scratch_plan)
            {
                Ok(scratch) => scratch,
                Err(cause) => {
                    drop(batches);
                    return NodeSceneDraftBuild::failure(prepare_node_post_evidence_failure(
                        assembly, evidence, &plans, cause,
                    ));
                }
            };
        let geometry_refs = match retain_geometry(
            &mut assembly,
            &plans.geometry,
            geometry_scratch,
            &batches.geometry,
            &batches.nodes,
            nodes_ref,
            appearance_refs.materials(),
            keys,
            &mut work,
            &mut faults,
            plans.build_invariant,
        ) {
            Ok(geometry) => geometry,
            Err(cause) => {
                drop(batches);
                return NodeSceneDraftBuild::failure(prepare_node_post_evidence_failure(
                    assembly, evidence, &plans, cause,
                ));
            }
        };
        let deformation_refs = match retain_deformation(
            &mut assembly,
            &plans.deformation,
            &batches.deformation,
            &batches.nodes,
            DeformationDependencyTableRefs::new(
                nodes_ref,
                geometry_refs.logical_meshes(),
                geometry_refs.primitives(),
                geometry_refs.mesh_instances(),
            ),
            keys,
            &mut work,
            plans.build_invariant,
        ) {
            Ok(deformation) => deformation,
            Err(cause) => {
                drop(batches);
                return NodeSceneDraftBuild::failure(prepare_node_post_evidence_failure(
                    assembly, evidence, &plans, cause,
                ));
            }
        };
        let animation_refs = match retain_animation(
            &mut assembly,
            &plans.animation,
            &batches.animations,
            AnimationTargetTableRefs::new(nodes_ref, deformation_refs.mesh_instances()),
            keys,
            &mut work,
            plans.build_invariant,
        ) {
            Ok(animation) => animation,
            Err(cause) => {
                drop(batches);
                return NodeSceneDraftBuild::failure(prepare_node_post_evidence_failure(
                    assembly, evidence, &plans, cause,
                ));
            }
        };
        let directory = match retain_node_scene_directory(
            &mut assembly,
            plans.directory,
            NodeSceneDirectoryRefs {
                nodes: nodes_ref,
                references,
                transforms,
            },
            SceneDomainDirectoryRefs {
                appearance: appearance_refs.directory(),
                geometry: geometry_refs.directory(),
                deformation: deformation_refs.directory(),
                animation: animation_refs.directory(),
            },
            &plans,
            keys,
            &mut work,
        ) {
            Ok(directory) => directory,
            Err(cause) => {
                drop(batches);
                return NodeSceneDraftBuild::failure(prepare_node_post_evidence_failure(
                    assembly, evidence, &plans, cause,
                ));
            }
        };
        let evidence_counts = plans.evidence_counts();
        let deformation_uniqueness_capacity = batches.deformation.maximum_uniqueness_capacity();
        let draft = NodeSceneDraft { brand, directory };
        let owner = PendingNodeSceneDraft {
            assembly,
            brand,
            keys,
            expected_directory: directory,
            evidence,
            evidence_counts,
            work,
            deformation_uniqueness_capacity,
        };
        // No source authoring slice or compiled validation plan crosses the draft boundary.
        drop(batches);
        Ok(NodeSceneDraftBuild::Draft { draft, owner })
    }
}

fn try_plan_geometry_reference_scratch<R>(
    assembly: &mut SceneStoreAssembly,
    geometry: &ValidatedGeometryBatch<'_, R>,
) -> Result<GeometryReferenceScratchPlan, StoreRuntimeError> {
    let mut checkpoint = assembly.try_control_checkpoint()?;
    GeometryReferenceScratchPlan::try_new(&mut checkpoint, geometry)
}

fn try_create_geometry_reference_scratch(
    assembly: &mut SceneStoreAssembly,
    keys: SceneStoreKeys,
    plan: GeometryReferenceScratchPlan,
) -> Result<GeometryReferenceScratch, StoreRuntimeError> {
    let mut checkpoint = assembly.try_control_checkpoint()?;
    GeometryReferenceScratch::try_new(&mut checkpoint, keys, plan)
}

fn retain_node_scene_evidence(
    assembly: &mut SceneStoreAssembly,
    plans: &NodeSceneMaterializationPlans,
) -> Result<OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>, StoreRuntimeError> {
    retain_node_scene_evidence_counts(assembly, plans.evidence, plans.evidence_counts())
}

fn retain_node_scene_evidence_counts(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    counts: SceneEvidenceCounts,
) -> Result<OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>, StoreRuntimeError> {
    assembly.try_retain_outcome_evidence_records(plan, NodeSceneOutcomeEvidenceSchema, |writer| {
        writer.write_row(|row| {
            row.write_scalar(SCENE_SCHEMA_REVISION)?;
            row.write_scalar(NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE)?;
            row.write_scalar(counts.canonical_block_count)?;
            row.write_scalar(counts.node_count)?;
            row.write_scalar(counts.image_count)?;
            row.write_scalar(counts.sampler_count)?;
            row.write_scalar(counts.texture_count)?;
            row.write_scalar(counts.material_count)?;
            row.write_scalar(counts.material_binding_count)?;
            row.write_scalar(counts.logical_mesh_count)?;
            row.write_scalar(counts.primitive_count)?;
            row.write_scalar(counts.attribute_count)?;
            row.write_scalar(counts.mesh_instance_count)?;
            row.write_scalar(counts.skin_count)?;
            row.write_scalar(counts.logical_morph_count)?;
            row.write_scalar(counts.morph_definition_count)?;
            row.write_scalar(counts.primitive_deformation_count)?;
            row.write_scalar(counts.morph_target_count)?;
            row.write_scalar(counts.morph_delta_count)?;
            row.write_scalar(counts.animation_clip_count)?;
            row.write_scalar(counts.animation_channel_count)?;
            row.write_scalar(counts.payload_block_count)
        })
    })
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
    domains: SceneDomainDirectoryRefs,
    plans: &NodeSceneMaterializationPlans,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
) -> Result<CanonicalBlockRef<SceneDirectorySchema>, StoreRuntimeError> {
    let failure = plans.build_invariant;
    let NodeSceneDirectoryRefs {
        nodes,
        references,
        transforms,
    } = refs;
    let SceneDomainDirectoryRefs {
        appearance,
        geometry,
        deformation,
        animation,
    } = domains;
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            SceneDirectorySchema,
            |writer, control| {
                work.observe(control, 1)?;
                writer
                    .write_row(|row| {
                        row.write_scalar(SCENE_SCHEMA_REVISION)?;
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
                        row.write_scalar(plans.source_root_count)?;
                        row.write_record_ref(appearance.into_store())?;
                        row.write_record_ref(geometry.into_store())?;
                        row.write_record_ref(deformation.into_store())?;
                        row.write_record_ref(animation.into_store())
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
) -> Result<PreparedNodeSceneFailure, SceneConstructionError> {
    match post_evidence_failure_code(&cause) {
        Some(cause) => prepare_node_rich_failure_from_assembly(
            assembly,
            evidence,
            plans.evidence_counts(),
            cause,
        ),
        None => {
            prepare_node_minimal_fallback_from_assembly(assembly, node_scene_failure_code(&cause))
        }
    }
}
