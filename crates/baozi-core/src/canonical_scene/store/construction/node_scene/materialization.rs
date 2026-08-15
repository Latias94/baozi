use std::num::NonZeroU64;

use baozi_operation::{
    CanonicalRecordRowWriter, OutcomeEvidenceRecordRef, RecordBlock, RecordPlan,
    RetainedStoreBatchLimits, RetainedStoreBlockTablePlan, RetainedStorePublicationPlan,
    RetainedStoreReservation, RetainedStoreWriterPlan, SequencePlan, StoreBuildError,
    StoreControlledError, StorePlanError, StoreRuntimeError,
};

use crate::SceneValidationLimits;
#[cfg(test)]
use crate::canonical_scene::store::production::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, DeformationAuthoringBatch,
    GeometryAuthoringBatch, LineageAuthoringBatch, MetadataAuthoringBatch, NodeAuthoringBatch,
    NodeLocalId, OwnerProvenanceAuthoring, ProfileAuthoringBatch, ProfileOwner, ProvenanceLocalId,
    ProvenanceNodeAuthoring, ProvenanceOperationAuthoring, SpatialAuthoringBatch,
};
use crate::canonical_scene::store::production::{
    AnimationTargetCatalog, OperationAuthoringControl, Presence, ProfileOwnerCatalog,
    SceneAuthoringBatch, ValidatedAnimationBatch, ValidatedAppearanceBatch,
    ValidatedDeformationBatch, ValidatedGeometryBatch, ValidatedNodeBatch, ValidatedProfileBatch,
    ValidatedSpatialBatch, animation_limits_after_deformation, appearance_limits_after_nodes,
    deformation_limits_after_geometry, geometry_limits_after_appearance, node_limits_after_spatial,
    profile_limits_after_animation, validate_animation_batch, validate_appearance_batch,
    validate_deformation_batch, validate_geometry_batch, validate_node_batch,
    validate_profile_batch, validate_scene_aggregate, validate_spatial_batch,
};
use crate::canonical_scene::store::schema::animation::AnimationDirectorySchema;
use crate::canonical_scene::store::schema::appearance::AppearanceDirectorySchema;
use crate::canonical_scene::store::schema::deformation::DeformationDirectorySchema;
use crate::canonical_scene::store::schema::spatial::{
    SpatialCameraTableSchema, SpatialDirectorySchema, SpatialLightTableSchema,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, NODE_SCENE_CHILD_REFERENCE_FIELD_COUNT,
    NODE_SCENE_EVIDENCE_FIELD_COUNT, NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE,
    NODE_SCENE_MATRIX_SCALAR_COUNT, NODE_SCENE_NODE_FIELD_COUNT, NODE_SCENE_TRANSFORM_MATRIX,
    NODE_SCENE_TRANSFORM_SYNTHETIC, NODE_SCENE_TRANSFORM_TRS, NODE_SCENE_TRS_SCALAR_COUNT,
    NodeSceneAuthoredTransformSchema, NodeSceneChildReferenceTableSchema, NodeSceneNodeTableSchema,
    NodeSceneOutcomeEvidenceSchema, PRESENCE_ABSENT, PRESENCE_PRESENT, SCENE_DIRECTORY_FIELD_COUNT,
    SCENE_SCHEMA_REVISION, SceneDirectorySchema, SceneGeometryDirectorySchema,
};
use crate::entity::AuthoredTransformKind;
use crate::identity::{ReservedSceneGeneration, SceneGenerationError, SceneGenerationId};
#[cfg(test)]
use crate::provenance::{OperationKind, OptionsDigest};
#[cfg(test)]
use crate::scene_space::SceneSpace;

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
use super::profile::{ProfileMaterializationPlans, retain_profile};
use super::publication::{
    node_scene_failure_code, post_evidence_failure_code,
    prepare_node_minimal_fallback_from_assembly, prepare_node_minimal_fallback_from_open_recovery,
    prepare_node_minimal_fallback_from_operation, prepare_node_rich_failure_from_assembly,
};
mod spatial;
use super::{
    FreshNodeSceneOwner, NODE_SCENE_BUILD_INVARIANT_CODE, NODE_SCENE_COMMITMENT_REJECTION_CODE,
    NodeSceneBuildFaults, NodeSceneConstructionRejection, NodeSceneDraft, NodeSceneDraftBuild,
    NodeSceneWorkMeter, PendingNodeSceneDraft, PreparedNodeSceneConstruction,
    PreparedNodeSceneFailure, RejectedNodeSceneConstruction, SceneEvidenceCounts,
    ValidatedNodeSceneConstruction,
};
use spatial::{RetainedSpatialRefs, SpatialMaterializationPlans, retain_spatial};

#[cfg(test)]
const TEST_PROFILE_MAX_NODES: usize = 64;
#[cfg(test)]
const TEST_PROFILE_PROVENANCE: [ProvenanceLocalId; 1] = [ProvenanceLocalId::new(0)];
#[cfg(test)]
static TEST_PROFILE_OPERATION_KIND: OperationKind = OperationKind::Generated;
#[cfg(test)]
static TEST_PROFILE_PROVENANCE_NODES: [ProvenanceNodeAuthoring<'static>; 1] =
    [ProvenanceNodeAuthoring::operation(
        ProvenanceOperationAuthoring::new(
            &TEST_PROFILE_OPERATION_KIND,
            OptionsDigest::sha256([0; 32]),
            &[],
            &[],
        ),
    )];
#[cfg(test)]
static TEST_PROFILE_OWNER_PROVENANCE: [OwnerProvenanceAuthoring<'static>;
    TEST_PROFILE_MAX_NODES + 2] = test_profile_owner_provenance();

#[cfg(test)]
const fn test_profile_owner_provenance()
-> [OwnerProvenanceAuthoring<'static>; TEST_PROFILE_MAX_NODES + 2] {
    let mut rows = [OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &TEST_PROFILE_PROVENANCE);
        TEST_PROFILE_MAX_NODES + 2];
    rows[1] = OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, &TEST_PROFILE_PROVENANCE);
    let mut node = 0;
    while node < TEST_PROFILE_MAX_NODES {
        rows[node + 2] = OwnerProvenanceAuthoring::new(
            ProfileOwner::Node(NodeLocalId::new(node as u32)),
            &TEST_PROFILE_PROVENANCE,
        );
        node += 1;
    }
    rows
}

#[cfg(test)]
fn test_node_profile(node_count: usize) -> ProfileAuthoringBatch<'static> {
    assert!(
        node_count <= TEST_PROFILE_MAX_NODES,
        "node-only test profile supports at most {TEST_PROFILE_MAX_NODES} authored nodes"
    );
    ProfileAuthoringBatch::new(
        LineageAuthoringBatch::new(
            &[],
            &[],
            &TEST_PROFILE_PROVENANCE_NODES,
            &TEST_PROFILE_OWNER_PROVENANCE[..node_count + 2],
        ),
        MetadataAuthoringBatch::empty(),
    )
}

#[cfg(test)]
fn test_scene_authoring_batch<'input>(
    input: NodeAuthoringBatch<'input>,
) -> SceneAuthoringBatch<'input> {
    let profile = test_node_profile(input.nodes().len());
    SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
        input,
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::empty(),
        DeformationAuthoringBatch::empty(),
        AnimationAuthoringBatch::empty(),
        profile,
    )
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(super) enum NodeScenePlanningFault {
    SealedPlan,
    Publication,
}

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
        self.try_validate_scene_batch(test_scene_authoring_batch(input), limits)
    }

    #[cfg(test)]
    pub(super) fn try_validate_node_batch_with_planning_fault<'input>(
        self,
        input: NodeAuthoringBatch<'input>,
        limits: SceneValidationLimits,
        planning_fault: NodeScenePlanningFault,
    ) -> Result<ValidatedNodeSceneConstruction<'input>, RejectedNodeSceneConstruction> {
        let (operation, batches) =
            self.try_validate_scene_batches(test_scene_authoring_batch(input), limits)?;
        match planning_fault {
            NodeScenePlanningFault::SealedPlan => {
                drop(batches);
                Err(planning_rejection(
                    operation,
                    StorePlanError::InvalidBlockTable,
                ))
            }
            NodeScenePlanningFault::Publication => {
                let plan = match NodeSceneSealedPlan::try_new(batches) {
                    Ok(plan) => plan,
                    Err(failure) => return Err(planning_rejection(operation, failure)),
                };
                Err(reject_bound_plan(
                    plan,
                    operation,
                    StorePlanError::InvalidBlockTable,
                ))
            }
        }
    }

    fn try_validate_scene_batch<'input>(
        self,
        input: SceneAuthoringBatch<'input>,
        limits: SceneValidationLimits,
    ) -> Result<ValidatedNodeSceneConstruction<'input>, RejectedNodeSceneConstruction> {
        let (operation, batches) = self.try_validate_scene_batches(input, limits)?;
        seal_node_scene_plan(operation, batches)
    }

    fn try_validate_scene_batches<'input>(
        mut self,
        input: SceneAuthoringBatch<'input>,
        limits: SceneValidationLimits,
    ) -> Result<
        (
            SceneConstructionOperation,
            ValidatedNodeSceneBatches<'input, RetainedStoreReservation<SceneConstructionOwner>>,
        ),
        RejectedNodeSceneConstruction,
    > {
        let keys = self.keys;
        let spatial_validation = {
            let checkpoint = self.raw.control_checkpoint();
            let mut control =
                OperationAuthoringControl::new(checkpoint, keys.phase, keys.dimension);
            validate_spatial_batch(input.spatial(), input.nodes(), limits, &mut control)
        };
        let spatial = match spatial_validation {
            Ok(spatial) => spatial,
            Err(failure) => {
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::SpatialAuthoring(failure),
                });
            }
        };
        let node_limits = match node_limits_after_spatial(&spatial, limits) {
            Ok(node_limits) => node_limits,
            Err(failure) => {
                drop(spatial);
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::Aggregate(failure),
                });
            }
        };
        let validation = {
            let checkpoint = self.raw.control_checkpoint();
            let mut control =
                OperationAuthoringControl::new(checkpoint, keys.phase, keys.dimension);
            validate_node_batch(input.nodes(), node_limits, &mut control)
        };
        let nodes = match validation {
            Ok(nodes) => nodes,
            Err(failure) => {
                drop(spatial);
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::NodeAuthoring(failure),
                });
            }
        };
        let appearance_limits = match appearance_limits_after_nodes(&nodes, node_limits) {
            Ok(appearance_limits) => appearance_limits,
            Err(failure) => {
                drop(nodes);
                drop(spatial);
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
                drop(spatial);
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
                drop(spatial);
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
                drop(spatial);
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
                drop(spatial);
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
                drop(spatial);
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
                    drop(spatial);
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
                drop(spatial);
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::AnimationAuthoring(failure),
                });
            }
        };
        let profile_limits = match profile_limits_after_animation(&animations, animation_limits) {
            Ok(profile_limits) => profile_limits,
            Err(failure) => {
                drop(animations);
                drop(deformation);
                drop(geometry);
                drop(appearance);
                drop(nodes);
                drop(spatial);
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::Aggregate(failure),
                });
            }
        };
        let owners = match ProfileOwnerCatalog::from_validated(
            &spatial,
            &nodes,
            &appearance,
            &geometry,
            &deformation,
            &animations,
        ) {
            Ok(owners) => owners,
            Err(failure) => {
                drop(animations);
                drop(deformation);
                drop(geometry);
                drop(appearance);
                drop(nodes);
                drop(spatial);
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::ProfileOwnerCatalog(failure),
                });
            }
        };
        let profile_validation = {
            let checkpoint = self.raw.control_checkpoint();
            let mut control =
                OperationAuthoringControl::new(checkpoint, keys.phase, keys.dimension);
            validate_profile_batch(input.profile(), owners, profile_limits, &mut control)
        };
        let profile = match profile_validation {
            Ok(profile) => profile,
            Err(failure) => {
                drop(animations);
                drop(deformation);
                drop(geometry);
                drop(appearance);
                drop(nodes);
                drop(spatial);
                return Err(RejectedNodeSceneConstruction {
                    operation: self,
                    failure: NodeSceneConstructionRejection::ProfileAuthoring(failure),
                });
            }
        };
        if let Err(failure) = validate_scene_aggregate(
            &spatial,
            &nodes,
            &appearance,
            &geometry,
            &deformation,
            &animations,
            &profile,
            limits,
        ) {
            drop(profile);
            drop(animations);
            drop(deformation);
            drop(geometry);
            drop(appearance);
            drop(nodes);
            drop(spatial);
            return Err(RejectedNodeSceneConstruction {
                operation: self,
                failure: NodeSceneConstructionRejection::Aggregate(failure),
            });
        }
        let batches = ValidatedNodeSceneBatches {
            profile,
            animations,
            deformation,
            geometry,
            appearance,
            nodes,
            spatial,
        };
        Ok((self, batches))
    }
}

fn planning_rejection(
    operation: SceneConstructionOperation,
    failure: StorePlanError,
) -> RejectedNodeSceneConstruction {
    RejectedNodeSceneConstruction {
        operation,
        failure: NodeSceneConstructionRejection::Planning(failure),
    }
}

fn seal_node_scene_plan<'input>(
    operation: SceneConstructionOperation,
    batches: ValidatedNodeSceneBatches<'input, RetainedStoreReservation<SceneConstructionOwner>>,
) -> Result<ValidatedNodeSceneConstruction<'input>, RejectedNodeSceneConstruction> {
    let plan = match NodeSceneSealedPlan::try_new(batches) {
        Ok(plan) => plan,
        Err(failure) => return Err(planning_rejection(operation, failure)),
    };
    let bound = BoundNodeScenePlan::try_bind(plan, operation)?;
    Ok(ValidatedNodeSceneConstruction { bound })
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
    spatial: CanonicalBlockRef<SpatialDirectorySchema>,
    appearance: CanonicalBlockRef<AppearanceDirectorySchema>,
    geometry: CanonicalBlockRef<SceneGeometryDirectorySchema>,
    deformation: CanonicalBlockRef<DeformationDirectorySchema>,
    animation: CanonicalBlockRef<AnimationDirectorySchema>,
    profile:
        CanonicalBlockRef<crate::canonical_scene::store::schema::profile::ProfileDirectorySchema>,
}

struct ValidatedNodeSceneBatches<'input, R> {
    profile: ValidatedProfileBatch<'input, R>,
    animations: ValidatedAnimationBatch<'input, R>,
    deformation: ValidatedDeformationBatch<'input, R>,
    geometry: ValidatedGeometryBatch<'input, R>,
    appearance: ValidatedAppearanceBatch<'input, R>,
    nodes: ValidatedNodeBatch<'input, R>,
    spatial: ValidatedSpatialBatch<'input, R>,
}

/// Complete operation-neutral plan produced by bounded authoring preflight.
///
/// The plan owns every validated collection and scratch reservation together with the exact
/// materialization shape derived from those collections. Operation-specific publication keys stay
/// outside this value and are bound by the consuming session adapter before canonical storage opens.
pub(super) struct NodeSceneSealedPlan<'input, R> {
    batches: ValidatedNodeSceneBatches<'input, R>,
    materialization: NodeSceneMaterializationPlans,
    semantic_work: NonZeroU64,
}

/// Legacy operation binding for one exact sealed plan.
///
/// Publication is derived while the operation and plan are consumed together. Keeping the fields
/// private to this module prevents a sibling operation's publication plan from being substituted
/// while the legacy construction rail remains in use.
pub(super) struct BoundNodeScenePlan<'input> {
    plan: NodeSceneSealedPlan<'input, RetainedStoreReservation<SceneConstructionOwner>>,
    publication: RetainedStorePublicationPlan,
    // The operation owner must outlive every compiled-plan reservation above.
    owner: FreshNodeSceneOwner,
}

struct NodeSceneMaterializationPlans {
    evidence: RecordPlan,
    transforms: Option<SequencePlan<f32>>,
    references: Option<RecordPlan>,
    nodes: RecordPlan,
    spatial: SpatialMaterializationPlans,
    appearance: AppearanceMaterializationPlans,
    geometry: GeometryMaterializationPlans,
    deformation: DeformationMaterializationPlans,
    animation: AnimationMaterializationPlans,
    profile: ProfileMaterializationPlans,
    directory: RecordPlan,
    canonical_block_count: u32,
    node_count: u32,
    source_node_count: u32,
    source_root_count: u32,
    transform_scalar_count: u32,
    payload_block_count: u32,
    shareable_leaf_block_count: u32,
    build_invariant: StoreBuildError,
}

impl<'input, R> NodeSceneSealedPlan<'input, R> {
    fn try_new(batches: ValidatedNodeSceneBatches<'input, R>) -> Result<Self, StorePlanError> {
        let materialization = NodeSceneMaterializationPlans::new(&batches)?;
        let semantic_work = validated_scene_semantic_work(&batches)?;
        Ok(Self {
            batches,
            materialization,
            semantic_work,
        })
    }

    fn publication_plan(
        &self,
        keys: SceneStoreKeys,
    ) -> Result<RetainedStorePublicationPlan, StorePlanError> {
        self.materialization.publication_plan(keys)
    }

    #[allow(
        dead_code,
        reason = "consumed by the import-bound activation slice in U4e"
    )]
    pub(super) const fn semantic_work(&self) -> NonZeroU64 {
        self.semantic_work
    }

    fn into_parts(
        self,
    ) -> (
        ValidatedNodeSceneBatches<'input, R>,
        NodeSceneMaterializationPlans,
    ) {
        (self.batches, self.materialization)
    }
}

impl<'input> BoundNodeScenePlan<'input> {
    fn try_bind(
        plan: NodeSceneSealedPlan<'input, RetainedStoreReservation<SceneConstructionOwner>>,
        operation: SceneConstructionOperation,
    ) -> Result<Self, RejectedNodeSceneConstruction> {
        let keys = operation.keys;
        let publication = match plan.publication_plan(keys) {
            Ok(publication) => publication,
            Err(failure) => {
                return Err(reject_bound_plan(plan, operation, failure));
            }
        };
        let SceneConstructionOperation { raw, brand, keys } = operation;
        Ok(Self {
            plan,
            publication,
            owner: FreshNodeSceneOwner { raw, brand, keys },
        })
    }

    pub(super) const fn semantic_work(&self) -> NonZeroU64 {
        self.plan.semantic_work()
    }

    fn into_parts(
        self,
    ) -> (
        NodeSceneSealedPlan<'input, RetainedStoreReservation<SceneConstructionOwner>>,
        RetainedStorePublicationPlan,
        FreshNodeSceneOwner,
    ) {
        (self.plan, self.publication, self.owner)
    }
}

fn reject_bound_plan(
    plan: NodeSceneSealedPlan<'_, RetainedStoreReservation<SceneConstructionOwner>>,
    operation: SceneConstructionOperation,
    failure: StorePlanError,
) -> RejectedNodeSceneConstruction {
    drop(plan);
    planning_rejection(operation, failure)
}

impl NodeSceneMaterializationPlans {
    fn evidence_counts(&self) -> SceneEvidenceCounts {
        let profile = self.profile.evidence_counts();
        SceneEvidenceCounts {
            canonical_block_count: self.canonical_block_count,
            node_count: self.node_count,
            camera_count: self.spatial.camera_count(),
            light_count: self.spatial.light_count(),
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
            profile_segment_count: profile.segments,
            profile_resource_count: profile.resources,
            profile_source_count: profile.sources,
            profile_provenance_count: profile.provenance,
            profile_operation_count: profile.operations,
            profile_parent_count: profile.parents,
            profile_owner_count: profile.owners,
            profile_attachment_count: profile.attachments,
            profile_metadata_entry_count: profile.metadata_entries,
            shareable_leaf_block_count: self.shareable_leaf_block_count,
        }
    }

    fn new<R>(batches: &ValidatedNodeSceneBatches<'_, R>) -> Result<Self, StorePlanError> {
        let spatial_batch = &batches.spatial;
        let nodes_batch = &batches.nodes;
        let appearance_batch = &batches.appearance;
        let geometry_batch = &batches.geometry;
        let deformation_batch = &batches.deformation;
        let animation_batch = &batches.animations;
        let profile_batch = &batches.profile;
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
        let spatial = SpatialMaterializationPlans::new(spatial_batch)?;
        let appearance = AppearanceMaterializationPlans::new(appearance_batch)?;
        let geometry = GeometryMaterializationPlans::new(geometry_batch)?;
        let deformation = DeformationMaterializationPlans::new(deformation_batch)?;
        let animation = AnimationMaterializationPlans::new(animation_batch)?;
        let profile = ProfileMaterializationPlans::new(profile_batch)?;
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
            .and_then(|count| count.checked_add(spatial.canonical_block_count()))
            .and_then(|count| count.checked_add(appearance.canonical_block_count()))
            .and_then(|count| count.checked_add(geometry.canonical_block_count()))
            .and_then(|count| count.checked_add(deformation.canonical_block_count()))
            .and_then(|count| count.checked_add(animation.canonical_block_count()))
            .and_then(|count| count.checked_add(profile.canonical_block_count()))
            .ok_or(StorePlanError::SizeOverflow)?;
        let canonical_block_count =
            u32::try_from(canonical_block_count_usize).map_err(|_| StorePlanError::SizeOverflow)?;
        let domain_payload_block_count = appearance
            .payload_block_count()
            .checked_add(spatial.payload_block_count())
            .and_then(|count| count.checked_add(geometry.payload_block_count()))
            .and_then(|count| count.checked_add(deformation.payload_block_count()))
            .and_then(|count| count.checked_add(animation.payload_block_count()))
            .ok_or(StorePlanError::SizeOverflow)?;
        let payload_block_count = domain_payload_block_count
            .checked_add(profile.payload_block_count())
            .ok_or(StorePlanError::SizeOverflow)?;
        let shareable_leaf_block_count = domain_payload_block_count
            .checked_add(profile.shareable_leaf_block_count())
            .ok_or(StorePlanError::SizeOverflow)?;
        let build_invariant = StoreBuildError::domain(NODE_SCENE_BUILD_INVARIANT_CODE)?;

        Ok(Self {
            evidence,
            transforms,
            references,
            nodes,
            spatial,
            appearance,
            geometry,
            deformation,
            animation,
            profile,
            directory,
            canonical_block_count,
            node_count,
            source_node_count,
            source_root_count,
            transform_scalar_count,
            payload_block_count,
            shareable_leaf_block_count,
            build_invariant,
        })
    }

    fn publication_plan(
        &self,
        keys: SceneStoreKeys,
    ) -> Result<RetainedStorePublicationPlan, StorePlanError> {
        let canonical_block_count = usize::try_from(self.canonical_block_count)
            .map_err(|_| StorePlanError::SizeOverflow)?;
        let evidence_block_count = 1_usize;
        let canonical_writer = node_scene_writer_plan(
            keys,
            canonical_block_count
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
            canonical_block_count,
            canonical_block_count,
        )?;
        let evidence_table = RetainedStoreBlockTablePlan::new(
            evidence_writer,
            evidence_block_count,
            evidence_block_count,
        )?;
        Ok(RetainedStorePublicationPlan::success(
            canonical_table,
            evidence_table,
        ))
    }
}

fn validated_scene_semantic_work<R>(
    batches: &ValidatedNodeSceneBatches<'_, R>,
) -> Result<NonZeroU64, StorePlanError> {
    let work = [
        batches.spatial.validation_work(),
        batches.nodes.validation_work(),
        batches.appearance.validation_work(),
        batches.geometry.validation_work(),
        batches.deformation.validation_work(),
        batches.animations.validation_work(),
        batches.profile.validation_work(),
    ]
    .into_iter()
    .try_fold(0_u64, u64::checked_add)
    .ok_or(StorePlanError::SizeOverflow)?;
    NonZeroU64::new(work).ok_or(StorePlanError::InvalidBlockTable)
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
        let Self { bound } = self;
        let (plan, publication, owner) = bound.into_parts();
        let (batches, plans) = plan.into_parts();
        let FreshNodeSceneOwner { raw, brand, keys } = owner;
        let mut assembly: SceneStoreAssembly = match raw.try_publication_assembly(publication) {
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
        let spatial_refs = match retain_spatial(
            &mut assembly,
            &plans.spatial,
            &batches.spatial,
            keys,
            &mut work,
            plans.build_invariant,
        ) {
            Ok(spatial) => spatial,
            Err(cause) => {
                drop(batches);
                return NodeSceneDraftBuild::failure(prepare_node_post_evidence_failure(
                    assembly, evidence, &plans, cause,
                ));
            }
        };
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
        let node_payloads = RetainedNodePayloadRefs {
            transforms,
            spatial: spatial_refs,
        };
        let nodes_ref = match retain_node_scene_nodes(
            &mut assembly,
            plans.nodes,
            node_payloads,
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
        let profile_refs = match retain_profile(
            &mut assembly,
            &plans.profile,
            &batches.profile,
            keys,
            &mut work,
            &mut faults.profile,
            plans.build_invariant,
        ) {
            Ok(profile) => profile,
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
                spatial: spatial_refs.directory(),
                appearance: appearance_refs.directory(),
                geometry: geometry_refs.directory(),
                deformation: deformation_refs.directory(),
                animation: animation_refs.directory(),
                profile: profile_refs.directory(),
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
            row.write_scalar(counts.camera_count)?;
            row.write_scalar(counts.light_count)?;
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
            row.write_scalar(counts.payload_block_count)?;
            row.write_scalar(counts.profile_segment_count)?;
            row.write_scalar(counts.profile_resource_count)?;
            row.write_scalar(counts.profile_source_count)?;
            row.write_scalar(counts.profile_provenance_count)?;
            row.write_scalar(counts.profile_operation_count)?;
            row.write_scalar(counts.profile_parent_count)?;
            row.write_scalar(counts.profile_owner_count)?;
            row.write_scalar(counts.profile_attachment_count)?;
            row.write_scalar(counts.profile_metadata_entry_count)?;
            row.write_scalar(counts.shareable_leaf_block_count)
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

#[derive(Clone, Copy)]
struct RetainedNodePayloadRefs {
    transforms: Option<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>>,
    spatial: RetainedSpatialRefs,
}

fn retain_node_scene_nodes(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    payloads: RetainedNodePayloadRefs,
    batch: &ValidatedNodeBatch<'_, RetainedStoreReservation<SceneConstructionOwner>>,
    plans: &NodeSceneMaterializationPlans,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
) -> Result<CanonicalBlockRef<NodeSceneNodeTableSchema>, StoreRuntimeError> {
    let RetainedNodePayloadRefs {
        transforms,
        spatial,
    } = payloads;
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
                    row.write_scalar(0_u8)?;
                    write_node_spatial_attachment::<SpatialCameraTableSchema>(
                        row,
                        None,
                        spatial.cameras(),
                        failure,
                    )?;
                    write_node_spatial_attachment::<SpatialLightTableSchema>(
                        row,
                        None,
                        spatial.lights(),
                        failure,
                    )
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
                    let authored_node = authored_nodes.get(local).ok_or(failure)?;
                    let authored = authored_node.authored_transform();
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
                        row.write_sequence_ref(transforms.into_store())?;
                        write_node_spatial_attachment::<SpatialCameraTableSchema>(
                            row,
                            match authored_node.camera() {
                                Presence::Absent => None,
                                Presence::Present(camera) => Some(camera.index()),
                            },
                            spatial.cameras(),
                            failure,
                        )?;
                        write_node_spatial_attachment::<SpatialLightTableSchema>(
                            row,
                            match authored_node.light() {
                                Presence::Absent => None,
                                Presence::Present(light) => Some(light.index()),
                            },
                            spatial.lights(),
                            failure,
                        )
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
        spatial,
        appearance,
        geometry,
        deformation,
        animation,
        profile,
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
                        row.write_record_ref(spatial.into_store())?;
                        row.write_record_ref(appearance.into_store())?;
                        row.write_record_ref(geometry.into_store())?;
                        row.write_record_ref(deformation.into_store())?;
                        row.write_record_ref(animation.into_store())?;
                        row.write_record_ref(profile.into_store())
                    })
                    .map_err(StoreControlledError::from)
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn write_node_spatial_attachment<Tag>(
    row: &mut CanonicalRecordRowWriter<'_>,
    local: Option<u32>,
    table: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreBuildError>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    match local {
        Some(local) => {
            row.write_scalar(PRESENCE_PRESENT)?;
            row.write_scalar(local)?;
            row.write_record_ref(table.ok_or(failure)?.into_store())
        }
        None => {
            row.write_scalar(PRESENCE_ABSENT)?;
            row.write_scalar(0_u8)?;
            row.write_scalar(0_u8)
        }
    }
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
