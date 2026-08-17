use std::cell::Cell;
use std::num::NonZeroU64;

use baozi_operation::__canonical_activation::{
    CanonicalActivationOutcome, CanonicalActivationPolicy, CanonicalActivationSeal,
    CanonicalCentralValidationOutcome, CanonicalFailureRequirements, CanonicalLane,
    CanonicalMaterialization, CanonicalMaterializationRole, CanonicalRecordBuildRow,
    CanonicalSuccessRequirements, CanonicalTableShape, CanonicalValidationDecision,
    CanonicalValidationProgress, CanonicalValidationRole, ScopedRetainedStoreSessionOutcomeOwner,
    StoreActivationContainment,
};
use baozi_operation::{
    FinalProgress, OperationOwner, OutcomeEvidenceRecordRowWriter, ProgressPhase, ProgressUnit,
    RecordBlock, RecordPlan, RetainedStoreBatchLimits, RetainedStoreOperation,
    RetainedStoreWriterPlan, SequencePlan, StoreBuildError, StoreControlledError,
    StoreDimensionKey, StorePhaseKey, StorePlanError, StoreRuntimeError, StoredScalar, Utf8Plan,
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
    AnimationTargetCatalog, CanonicalAuthoringControl, Presence, ProfileOwnerCatalog,
    SceneAuthoringBatch, SceneInputEnvelope, ScopedAuthoringControl, ValidatedAnimationBatch,
    ValidatedAppearanceBatch, ValidatedDeformationBatch, ValidatedGeometryBatch,
    ValidatedNodeBatch, ValidatedProfileBatch, ValidatedSpatialBatch,
    animation_limits_after_deformation, appearance_limits_after_nodes,
    deformation_limits_after_geometry, geometry_limits_after_appearance, node_limits_after_spatial,
    profile_limits_after_animation, validate_animation_batch_with_control,
    validate_appearance_batch_with_control, validate_deformation_batch_with_control,
    validate_geometry_batch_with_control, validate_node_batch_with_control,
    validate_profile_batch_with_control, validate_scene_aggregate,
    validate_spatial_batch_with_control,
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
    NodeSceneAuthoredTransformSchema, NodeSceneChildReferenceTableSchema, NodeSceneNodeNameSchema,
    NodeSceneNodeTableSchema, NodeSceneOutcomeEvidenceSchema, PRESENCE_ABSENT, PRESENCE_PRESENT,
    SCENE_DIRECTORY_FIELD_COUNT, SCENE_SCHEMA_REVISION, SceneDirectorySchema,
    SceneGeometryDirectorySchema,
};
use crate::entity::AuthoredTransformKind;
#[cfg(test)]
use crate::provenance::{OperationKind, OptionsDigest};
#[cfg(test)]
use crate::scene_space::SceneSpace;

use super::super::{
    SceneAuthoringOperation, SceneConstructionError, SceneConstructionStartError, SceneStoreKeys,
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
use super::publication::NodeActivationFailureCause;
use super::scratch::{NodeSceneValidationScratchPlan, ScopedNodeSceneValidationScratchOwner};
mod spatial;
use super::{
    DeferredNodeSceneDraftBuild, NODE_SCENE_BUILD_INVARIANT_CODE, NodeSceneBuildFaults,
    NodeSceneConstructionRejection, NodeSceneDraft, NodeSceneDraftBuild, PendingNodeSceneDraft,
    PendingNodeSceneFailure, SceneEvidenceCounts,
};
use spatial::{RetainedSpatialRefs, SpatialMaterializationPlans, retain_spatial};

use crate::canonical_scene::store::validation::validate_node_scene_canonical_graph_metered;

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
pub(super) fn test_scene_authoring_batch<'input>(
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

/// Core-owned, roleless request for one scoped NodeScene activation.
///
/// The request derives its writer envelopes from the complete producer batch and mints the private
/// construction brand retained by its execution payload. It carries no store key, account, ledger,
/// or publication authority; the consuming host maps the roleless limits to its own vocabulary
/// before it creates the scoped outcome owner. Exact canonical shape remains absent until semantic
/// preflight seals [`NodeSceneSealedPlan`].
#[must_use = "the NodeScene activation request must be bound to one scoped outcome owner"]
pub(in crate::canonical_scene::store) struct NodeSceneActivationRequest<'input> {
    execution: NodeSceneActivationExecution<'input>,
    policy: NodeSceneActivationPolicyRequirements,
}

/// Roleless writer and failure envelopes supplied by core to the operation-owning host.
#[must_use = "the NodeScene activation policy requirements must bind one scoped outcome owner"]
struct NodeSceneActivationPolicyRequirements {
    canonical_writer_limits: RetainedStoreBatchLimits,
    outcome_evidence_writer_limits: RetainedStoreBatchLimits,
    failure: CanonicalFailureRequirements,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneWriterRoles {
    phase: StorePhaseKey,
    capacity_dimension: StoreDimensionKey,
    attribution_dimension: StoreDimensionKey,
}

impl NodeSceneWriterRoles {
    pub(in crate::canonical_scene::store) const fn new(
        phase: StorePhaseKey,
        capacity_dimension: StoreDimensionKey,
        attribution_dimension: StoreDimensionKey,
    ) -> Self {
        Self {
            phase,
            capacity_dimension,
            attribution_dimension,
        }
    }

    const fn bind(self, limits: RetainedStoreBatchLimits) -> RetainedStoreWriterPlan {
        RetainedStoreWriterPlan::new(
            self.phase,
            self.capacity_dimension,
            self.attribution_dimension,
            limits,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneActivationRoles {
    canonical_writer: NodeSceneWriterRoles,
    outcome_evidence_writer: NodeSceneWriterRoles,
    materialization_work_dimension: StoreDimensionKey,
    materialization_scratch_dimension: StoreDimensionKey,
    validation_phase: StorePhaseKey,
    validation_work_dimension: StoreDimensionKey,
    validation_scratch_dimension: StoreDimensionKey,
    canonical_destination_dimension: StoreDimensionKey,
    outcome_evidence_destination_dimension: StoreDimensionKey,
    canonical_string_dimension: Option<StoreDimensionKey>,
}

impl NodeSceneActivationRoles {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::canonical_scene::store) const fn new(
        canonical_writer: NodeSceneWriterRoles,
        outcome_evidence_writer: NodeSceneWriterRoles,
        materialization_work_dimension: StoreDimensionKey,
        materialization_scratch_dimension: StoreDimensionKey,
        validation_phase: StorePhaseKey,
        validation_work_dimension: StoreDimensionKey,
        validation_scratch_dimension: StoreDimensionKey,
        canonical_destination_dimension: StoreDimensionKey,
        outcome_evidence_destination_dimension: StoreDimensionKey,
    ) -> Self {
        Self {
            canonical_writer,
            outcome_evidence_writer,
            materialization_work_dimension,
            materialization_scratch_dimension,
            validation_phase,
            validation_work_dimension,
            validation_scratch_dimension,
            canonical_destination_dimension,
            outcome_evidence_destination_dimension,
            canonical_string_dimension: None,
        }
    }

    pub(in crate::canonical_scene::store) const fn with_canonical_string_dimension(
        mut self,
        dimension: StoreDimensionKey,
    ) -> Self {
        self.canonical_string_dimension = Some(dimension);
        self
    }

    const fn bind(
        self,
        canonical_writer_limits: RetainedStoreBatchLimits,
        outcome_evidence_writer_limits: RetainedStoreBatchLimits,
    ) -> CanonicalActivationPolicy {
        let policy = CanonicalActivationPolicy::new(
            self.canonical_writer.bind(canonical_writer_limits),
            self.outcome_evidence_writer
                .bind(outcome_evidence_writer_limits),
            CanonicalMaterializationRole::new(
                self.materialization_work_dimension,
                self.materialization_scratch_dimension,
            ),
            CanonicalValidationRole::new(
                self.validation_phase,
                self.validation_work_dimension,
                self.validation_scratch_dimension,
            ),
            self.canonical_destination_dimension,
            self.outcome_evidence_destination_dimension,
        );
        match self.canonical_string_dimension {
            Some(dimension) => policy.with_semantic_bytes_dimension(dimension),
            None => policy,
        }
    }
}

/// Core execution payload, including its unforgeable brand, retained after policy binding.
#[must_use = "the NodeScene activation execution must run with its bound scoped outcome owner"]
struct NodeSceneActivationExecution<'input> {
    brand: super::super::ConstructionBrand,
    input: SceneAuthoringBatch<'input>,
    validation_limits: SceneValidationLimits,
    failure_evidence: NodeSceneFailureEvidencePlan,
}

impl<'input> NodeSceneActivationRequest<'input> {
    pub(in crate::canonical_scene::store) fn try_new(
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
    ) -> Result<Self, SceneConstructionStartError> {
        let envelope = SceneInputEnvelope::try_new(input, validation_limits)?;
        let failure_evidence = NodeSceneFailureEvidencePlan::try_new()?;
        let policy = NodeSceneActivationPolicyRequirements {
            canonical_writer_limits: RetainedStoreBatchLimits::new(
                envelope.terminal_attribution_slots(),
                4,
            )?,
            outcome_evidence_writer_limits: RetainedStoreBatchLimits::new(2, 4)?,
            failure: failure_evidence.requirements()?,
        };
        let brand = super::super::ConstructionBrand::reserve()?;
        Ok(Self {
            execution: NodeSceneActivationExecution {
                brand,
                input,
                validation_limits,
                failure_evidence,
            },
            policy,
        })
    }

    pub(in crate::canonical_scene::store) const fn canonical_writer_limits(
        &self,
    ) -> RetainedStoreBatchLimits {
        self.policy.canonical_writer_limits
    }

    pub(in crate::canonical_scene::store) fn activate<O, PP, U>(
        self,
        operation: RetainedStoreOperation<O, PP, U>,
        roles: NodeSceneActivationRoles,
        validation_progress: CanonicalValidationProgress<PP, U>,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<NodeSceneDraftBuild<O, PP, U>, SceneConstructionError>
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        let activation = self.bind_operation(operation, roles)?;
        activate_node_scene_scoped(activation, validation_progress, final_progress)
    }

    pub(in crate::canonical_scene::store) fn activate_deferred<O, PP, U>(
        self,
        operation: RetainedStoreOperation<O, PP, U>,
        roles: NodeSceneActivationRoles,
        validation_progress: CanonicalValidationProgress<PP, U>,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<DeferredNodeSceneDraftBuild<O, PP, U>, SceneConstructionError>
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        let activation = self.bind_operation(operation, roles)?;
        activate_node_scene_scoped_deferred(activation, validation_progress, final_progress)
    }

    fn bind_operation<O, PP, U>(
        self,
        operation: RetainedStoreOperation<O, PP, U>,
        roles: NodeSceneActivationRoles,
    ) -> Result<
        BoundNodeSceneActivation<'input, O, PP, U, baozi_operation::FreshStoreLineage>,
        StoreActivationContainment,
    >
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        let Self { execution, policy } = self;
        let (canonical_writer_limits, outcome_evidence_writer_limits, failure) =
            policy.into_parts();
        let policy = roles.bind(canonical_writer_limits, outcome_evidence_writer_limits);
        let owner = operation.into_scoped_session_outcome_owner(policy, failure)?;
        Ok(BoundNodeSceneActivation { owner, execution })
    }
}

/// One request's execution payload structurally paired with its bound scoped outcome owner.
struct BoundNodeSceneActivation<'input, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    owner: ScopedRetainedStoreSessionOutcomeOwner<O, PP, U, L>,
    execution: NodeSceneActivationExecution<'input>,
}

impl NodeSceneActivationPolicyRequirements {
    fn into_parts(
        self,
    ) -> (
        RetainedStoreBatchLimits,
        RetainedStoreBatchLimits,
        CanonicalFailureRequirements,
    ) {
        (
            self.canonical_writer_limits,
            self.outcome_evidence_writer_limits,
            self.failure,
        )
    }
}

fn validate_node_scene_batches<'input, C>(
    input: SceneAuthoringBatch<'input>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<
    ValidatedNodeSceneBatches<'input, C::Reservation>,
    NodeSceneConstructionRejection<C::Error>,
>
where
    C: CanonicalAuthoringControl,
{
    let spatial =
        validate_spatial_batch_with_control(input.spatial(), input.nodes(), limits, control)
            .map_err(NodeSceneConstructionRejection::SpatialAuthoring)?;
    let node_limits = node_limits_after_spatial(&spatial, limits)
        .map_err(NodeSceneConstructionRejection::Aggregate)?;
    let nodes = validate_node_batch_with_control(spatial.node_input(), node_limits, control)
        .map_err(NodeSceneConstructionRejection::NodeAuthoring)?;
    let appearance_limits = appearance_limits_after_nodes(&nodes, node_limits)
        .map_err(NodeSceneConstructionRejection::Aggregate)?;
    let appearance = validate_appearance_batch_with_control(
        input.appearance(),
        input.profile().resource_snapshots(),
        appearance_limits,
        control,
    )
    .map_err(NodeSceneConstructionRejection::AppearanceAuthoring)?;
    let geometry_limits = geometry_limits_after_appearance(&appearance, appearance_limits)
        .map_err(NodeSceneConstructionRejection::Aggregate)?;
    let geometry = validate_geometry_batch_with_control(
        input.geometry(),
        nodes.authored_node_count(),
        appearance.material_count(),
        geometry_limits,
        control,
    )
    .map_err(NodeSceneConstructionRejection::GeometryAuthoring)?;
    let deformation_limits = deformation_limits_after_geometry(&geometry, geometry_limits)
        .map_err(NodeSceneConstructionRejection::Aggregate)?;
    let deformation = validate_deformation_batch_with_control(
        input.deformation(),
        &nodes,
        &geometry,
        deformation_limits,
        control,
    )
    .map_err(NodeSceneConstructionRejection::DeformationAuthoring)?;
    let animation_limits = animation_limits_after_deformation(&deformation, deformation_limits)
        .map_err(NodeSceneConstructionRejection::Aggregate)?;
    let targets = AnimationTargetCatalog::new(&nodes, &deformation);
    let animations = validate_animation_batch_with_control(
        input.animations(),
        &targets,
        animation_limits,
        control,
    )
    .map_err(NodeSceneConstructionRejection::AnimationAuthoring)?;
    let profile_limits = profile_limits_after_animation(&animations, animation_limits)
        .map_err(NodeSceneConstructionRejection::Aggregate)?;
    let owners = ProfileOwnerCatalog::from_validated(
        &spatial,
        &nodes,
        &appearance,
        &geometry,
        &deformation,
        &animations,
    )
    .map_err(NodeSceneConstructionRejection::ProfileOwnerCatalog)?;
    let profile =
        validate_profile_batch_with_control(input.profile(), owners, profile_limits, control)
            .map_err(NodeSceneConstructionRejection::ProfileAuthoring)?;
    validate_scene_aggregate(
        &spatial,
        &nodes,
        &appearance,
        &geometry,
        &deformation,
        &animations,
        &profile,
        limits,
    )
    .map_err(NodeSceneConstructionRejection::Aggregate)?;
    Ok(ValidatedNodeSceneBatches {
        profile,
        animations,
        deformation,
        geometry,
        appearance,
        nodes,
        spatial,
    })
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn validate_node_scene_batches_for_test<'input, C>(
    input: SceneAuthoringBatch<'input>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<(), NodeSceneConstructionRejection<C::Error>>
where
    C: CanonicalAuthoringControl,
{
    validate_node_scene_batches(input, limits, control).map(drop)
}

fn validate_and_seal_node_scene_plan<'input, C>(
    input: SceneAuthoringBatch<'input>,
    limits: SceneValidationLimits,
    failure_evidence: NodeSceneFailureEvidencePlan,
    control: &mut C,
) -> Result<NodeSceneSealedPlan<'input, C::Reservation>, NodeSceneConstructionRejection<C::Error>>
where
    C: CanonicalAuthoringControl,
{
    let batches = validate_node_scene_batches(input, limits, control)?;
    NodeSceneSealedPlan::try_new(batches, failure_evidence)
        .map_err(NodeSceneConstructionRejection::Planning)
}

impl<'input> SceneAuthoringOperation<'input> {
    /// Runs the complete fresh Scene authoring transaction through the sole scoped activation rail.
    pub(in crate::canonical_scene::store) fn finish_draft(
        self,
    ) -> Result<NodeSceneDraftBuild, SceneConstructionError> {
        let Self { raw, keys, request } = self;
        request.activate(
            raw,
            node_scene_activation_roles(keys),
            CanonicalValidationProgress::new(
                super::super::SceneConstructionPhase::Validation,
                super::super::SceneConstructionUnit::WorkUnits,
            ),
            super::super::final_progress(),
        )
    }

    #[cfg(test)]
    pub(super) fn finish_draft_with_test_faults(
        self,
        faults: NodeSceneBuildFaults,
    ) -> Result<NodeSceneDraftBuild, SceneConstructionError> {
        self.finish_draft_with_faults(faults)
    }

    #[cfg(test)]
    pub(super) fn finish_draft_with_planning_fault_for_test(
        self,
        planning: NodeScenePlanningFault,
    ) -> Result<NodeSceneDraftBuild, SceneConstructionError> {
        self.finish_draft_with_faults(NodeSceneBuildFaults {
            planning: Some(planning),
            ..NodeSceneBuildFaults::none()
        })
    }

    fn finish_draft_with_faults(
        self,
        faults: NodeSceneBuildFaults,
    ) -> Result<NodeSceneDraftBuild, SceneConstructionError> {
        let Self { raw, keys, request } = self;
        let activation = request.bind_operation(raw, node_scene_activation_roles(keys))?;
        drive_node_scene_activation(
            activation,
            CanonicalValidationProgress::new(
                super::super::SceneConstructionPhase::Validation,
                super::super::SceneConstructionUnit::WorkUnits,
            ),
            super::super::final_progress(),
            faults,
        )
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
    execution: NodeSceneExecutionPlan<'input, R>,
    success_requirements: CanonicalSuccessRequirements,
}

struct NodeSceneExecutionPlan<'input, R> {
    batches: ValidatedNodeSceneBatches<'input, R>,
    materialization: NodeSceneMaterializationPlans,
    geometry_reference_scratch: GeometryReferenceScratchPlan,
    validation_scratch: NodeSceneValidationScratchPlan,
}

#[derive(Clone, Copy)]
struct NodeSceneFailureEvidencePlan {
    records: RecordPlan,
}

impl NodeSceneFailureEvidencePlan {
    fn try_new() -> Result<Self, StorePlanError> {
        Ok(Self {
            records: RecordPlan::new(1, NODE_SCENE_EVIDENCE_FIELD_COUNT)?,
        })
    }

    fn requirements(self) -> Result<CanonicalFailureRequirements, StorePlanError> {
        Ok(CanonicalFailureRequirements::new(CanonicalTableShape::new(
            1, 1,
        )?))
    }

    fn validation_work_units(self) -> Result<u64, StorePlanError> {
        record_validation_work(self.records)
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneMaterializedRefs {
    pub(in crate::canonical_scene::store) directory: CanonicalBlockRef<SceneDirectorySchema>,
    pub(in crate::canonical_scene::store) nodes: CanonicalBlockRef<NodeSceneNodeTableSchema>,
    pub(in crate::canonical_scene::store) references:
        Option<CanonicalBlockRef<NodeSceneChildReferenceTableSchema>>,
    pub(in crate::canonical_scene::store) transforms:
        Option<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>>,
    pub(in crate::canonical_scene::store) node_names:
        Option<CanonicalBlockRef<NodeSceneNodeNameSchema>>,
    pub(in crate::canonical_scene::store) spatial: RetainedSpatialRefs,
    pub(in crate::canonical_scene::store) appearance: super::appearance::RetainedAppearanceRefs,
    pub(in crate::canonical_scene::store) geometry: super::geometry::RetainedGeometryRefs,
    pub(in crate::canonical_scene::store) deformation: super::deformation::RetainedDeformationRefs,
    pub(in crate::canonical_scene::store) animation: super::animation::RetainedAnimationRefs,
    pub(in crate::canonical_scene::store) profile: super::profile::RetainedProfileRefs,
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneActivationOutput {
    pub(in crate::canonical_scene::store) draft: NodeSceneDraft,
    pub(in crate::canonical_scene::store) refs: NodeSceneMaterializedRefs,
    pub(in crate::canonical_scene::store) evidence_counts: SceneEvidenceCounts,
    pub(in crate::canonical_scene::store) validation_scratch: NodeSceneValidationScratchPlan,
}

struct NodeSceneMaterializationPlans {
    transforms: Option<SequencePlan<f32>>,
    node_names: Option<Utf8Plan>,
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
    node_name_byte_count: u32,
    canonical_string_bytes: u64,
    payload_block_count: u32,
    shareable_leaf_block_count: u32,
    build_invariant: StoreBuildError,
}

impl<'input, R> NodeSceneSealedPlan<'input, R> {
    fn try_new(
        batches: ValidatedNodeSceneBatches<'input, R>,
        failure_evidence: NodeSceneFailureEvidencePlan,
    ) -> Result<Self, StorePlanError> {
        let materialization = NodeSceneMaterializationPlans::new(&batches)?;
        let geometry_reference_scratch = GeometryReferenceScratchPlan::try_new(&batches.geometry)?;
        let validation_scratch = NodeSceneValidationScratchPlan::try_new(
            materialization.evidence_counts(),
            batches.deformation.maximum_uniqueness_capacity(),
            materialization.deformation.influence_row_offset_count(),
            materialization.deformation.influence_count(),
            materialization.animation.metered_counts(),
            materialization.profile.metered_counts()?,
        )?;
        let success_requirements =
            materialization.success_requirements(validation_scratch, failure_evidence)?;
        Ok(Self {
            execution: NodeSceneExecutionPlan {
                batches,
                materialization,
                geometry_reference_scratch,
                validation_scratch,
            },
            success_requirements,
        })
    }

    fn into_seal_parts(
        self,
    ) -> (
        NodeSceneExecutionPlan<'input, R>,
        CanonicalSuccessRequirements,
    ) {
        (self.execution, self.success_requirements)
    }
}

impl NodeSceneMaterializationPlans {
    fn success_requirements(
        &self,
        validation_scratch: NodeSceneValidationScratchPlan,
        failure_evidence: NodeSceneFailureEvidencePlan,
    ) -> Result<CanonicalSuccessRequirements, StorePlanError> {
        let canonical_block_count = usize::try_from(self.canonical_block_count)
            .map_err(|_| StorePlanError::SizeOverflow)?;
        let validation_work =
            NonZeroU64::new(self.validation_work_units(validation_scratch, failure_evidence)?)
                .ok_or(StorePlanError::InvalidBlockTable)?;
        CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(canonical_block_count, canonical_block_count)?,
            canonical_block_count,
            validation_work,
        )
        .map(|requirements| requirements.with_semantic_bytes(self.canonical_string_bytes))
    }

    fn validation_work_units(
        &self,
        validation_scratch: NodeSceneValidationScratchPlan,
        failure_evidence: NodeSceneFailureEvidencePlan,
    ) -> Result<u64, StorePlanError> {
        [
            failure_evidence.validation_work_units()?,
            optional_sequence_validation_work(self.transforms)?,
            optional_utf8_validation_work(self.node_names)?,
            optional_record_validation_work(self.references)?,
            record_validation_work(self.nodes)?,
            self.spatial.validation_work_units()?,
            self.appearance.validation_work_units()?,
            self.geometry.validation_work_units()?,
            self.deformation.validation_work_units()?,
            self.animation.validation_work_units()?,
            self.profile.validation_work_units()?,
            record_validation_work(self.directory)?,
            validation_scratch.initialization_work_units(),
            validation_scratch.semantic_projection_work_units(),
            u64::from(self.spatial.camera_count()),
            u64::from(self.spatial.light_count()),
        ]
        .into_iter()
        .try_fold(0_u64, |total, work| {
            total.checked_add(work).ok_or(StorePlanError::SizeOverflow)
        })
    }

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
        let source_node_count_usize = nodes_batch.authored_node_count();
        let source_node_count =
            u32::try_from(source_node_count_usize).map_err(|_| StorePlanError::SizeOverflow)?;
        let source_root_count = nodes_batch.source_root_count();
        let transform_scalar_count_u64 = nodes_batch
            .matrix_scalar_count()
            .checked_add(nodes_batch.trs_scalar_count())
            .ok_or(StorePlanError::SizeOverflow)?;
        let transform_scalar_count =
            u32::try_from(transform_scalar_count_u64).map_err(|_| StorePlanError::SizeOverflow)?;
        let transform_scalar_count_usize =
            usize::try_from(transform_scalar_count).map_err(|_| StorePlanError::SizeOverflow)?;
        let node_name_byte_count = u32::try_from(nodes_batch.node_name_byte_count())
            .map_err(|_| StorePlanError::SizeOverflow)?;
        let profile_shape = profile_batch.retention_shape();
        let canonical_string_bytes = [
            u64::from(node_name_byte_count),
            u64::from(spatial_batch.tangent_basis_schema_name_bytes()),
            u64::from(spatial_batch.tangent_encoding_schema_name_bytes()),
            u64::from(appearance_batch.media_type_byte_count()),
            u64::from(appearance_batch.source_key_byte_count()),
            u64::from(appearance_batch.material_name_byte_count()),
            u64::from(geometry_batch.logical_mesh_name_byte_count()),
            u64::from(geometry_batch.attribute_semantic_name_byte_count()),
            u64::from(deformation_batch.skin_name_byte_count()),
            u64::from(deformation_batch.morph_name_byte_count()),
            u64::from(profile_shape.lineage_text_bytes()),
            u64::from(profile_shape.metadata_text_bytes()),
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
        .ok_or(StorePlanError::SizeOverflow)?;

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
        let node_names = if node_name_byte_count == 0 {
            None
        } else {
            Some(Utf8Plan::new(
                usize::try_from(node_name_byte_count).map_err(|_| StorePlanError::SizeOverflow)?,
            )?)
        };

        let record_block_count = [Some(nodes), Some(directory), references]
            .into_iter()
            .flatten()
            .count();
        let sequence_block_count = [transforms].into_iter().flatten().count();
        let utf8_block_count = [node_names].into_iter().flatten().count();
        let canonical_block_count_usize = record_block_count
            .checked_add(sequence_block_count)
            .and_then(|count| count.checked_add(utf8_block_count))
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
            .and_then(|count| count.checked_add(u32::from(node_names.is_some())))
            .ok_or(StorePlanError::SizeOverflow)?;
        let payload_block_count = domain_payload_block_count
            .checked_add(profile.payload_block_count())
            .ok_or(StorePlanError::SizeOverflow)?;
        let shareable_leaf_block_count = domain_payload_block_count
            .checked_add(profile.shareable_leaf_block_count())
            .ok_or(StorePlanError::SizeOverflow)?;
        let build_invariant = StoreBuildError::domain(NODE_SCENE_BUILD_INVARIANT_CODE)?;

        Ok(Self {
            transforms,
            node_names,
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
            node_name_byte_count,
            canonical_string_bytes,
            payload_block_count,
            shareable_leaf_block_count,
            build_invariant,
        })
    }
}

fn record_validation_work(plan: RecordPlan) -> Result<u64, StorePlanError> {
    let cells = plan
        .rows()
        .checked_mul(plan.fields())
        .ok_or(StorePlanError::SizeOverflow)?;
    u64::try_from(cells).map_err(|_| StorePlanError::SizeOverflow)
}

fn optional_record_validation_work(plan: Option<RecordPlan>) -> Result<u64, StorePlanError> {
    plan.map(record_validation_work)
        .transpose()
        .map(|work| work.unwrap_or(0))
}

fn optional_sequence_validation_work<T: StoredScalar>(
    plan: Option<SequencePlan<T>>,
) -> Result<u64, StorePlanError> {
    plan.map(|plan| u64::try_from(plan.element_count()).map_err(|_| StorePlanError::SizeOverflow))
        .transpose()
        .map(|work| work.unwrap_or(0))
}

fn optional_utf8_validation_work(plan: Option<Utf8Plan>) -> Result<u64, StorePlanError> {
    plan.map(|plan| u64::try_from(plan.bytes()).map_err(|_| StorePlanError::SizeOverflow))
        .transpose()
        .map(|work| work.unwrap_or(0))
}

fn node_scene_activation_roles(keys: SceneStoreKeys) -> NodeSceneActivationRoles {
    NodeSceneActivationRoles::new(
        NodeSceneWriterRoles::new(keys.phase, keys.dimension, keys.dimension),
        NodeSceneWriterRoles::new(keys.phase, keys.dimension, keys.dimension),
        keys.dimension,
        keys.dimension,
        keys.phase,
        keys.dimension,
        keys.dimension,
        keys.dimension,
        keys.dimension,
    )
}

fn materialize_node_scene<'lane, R, O, PP, U, L>(
    mut lane: CanonicalLane<'lane, O, PP, U, L>,
    plan: &NodeSceneExecutionPlan<'_, R>,
    brand: super::super::ConstructionBrand,
    mut faults: NodeSceneBuildFaults,
) -> CanonicalMaterialization<
    'lane,
    NodeSceneActivationOutput,
    NodeSceneConstructionRejection<StoreRuntimeError>,
    RecordBlock,
    SceneDirectorySchema,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    macro_rules! materialize {
        ($expression:expr) => {
            match $expression {
                Ok(value) => value,
                Err(cause) => {
                    let rejection = NodeSceneConstructionRejection::Materialization(cause);
                    let stable_code = rejection.stable_code();
                    return lane.reject(rejection, stable_code);
                }
            }
        };
    }

    let NodeSceneExecutionPlan {
        batches,
        materialization: plans,
        geometry_reference_scratch,
        validation_scratch,
    } = plan;
    let failure = plans.build_invariant;
    let spatial = materialize!(retain_spatial(
        &mut lane,
        &plans.spatial,
        &batches.spatial,
        failure,
    ));
    let transforms = match plans.transforms {
        Some(plan) => Some(materialize!(retain_node_scene_transforms(
            &mut lane,
            plan,
            &batches.nodes,
            plans,
            &mut faults,
        ))),
        None => None,
    };
    let node_names = match plans.node_names {
        Some(plan) => Some(materialize!(retain_node_scene_names(
            &mut lane,
            plan,
            &batches.nodes,
            plans,
        ))),
        None => None,
    };
    let references = match plans.references {
        Some(plan) => Some(materialize!(retain_node_scene_references(
            &mut lane,
            plan,
            &batches.nodes,
            plans,
        ))),
        None => None,
    };
    let nodes = materialize!(retain_node_scene_nodes(
        &mut lane,
        plans.nodes,
        RetainedNodePayloadRefs {
            transforms,
            names: node_names,
            spatial,
        },
        &batches.nodes,
        plans,
    ));
    let appearance = materialize!(retain_appearance(
        &mut lane,
        &plans.appearance,
        &batches.appearance,
        failure,
    ));
    let geometry_scratch = materialize!(GeometryReferenceScratch::try_new(
        &mut lane,
        *geometry_reference_scratch,
    ));
    let geometry = materialize!(retain_geometry(
        &mut lane,
        &plans.geometry,
        geometry_scratch,
        &batches.geometry,
        &batches.nodes,
        nodes,
        appearance.materials(),
        &mut faults,
        failure,
    ));
    let deformation = materialize!(retain_deformation(
        &mut lane,
        &plans.deformation,
        &batches.deformation,
        &batches.nodes,
        DeformationDependencyTableRefs::new(
            nodes,
            geometry.logical_meshes(),
            geometry.primitives(),
            geometry.mesh_instances(),
        ),
        failure,
    ));
    let animation = materialize!(retain_animation(
        &mut lane,
        &plans.animation,
        &batches.animations,
        AnimationTargetTableRefs::new(nodes, deformation.mesh_instances()),
        failure,
    ));
    let profile = materialize!(retain_profile(
        &mut lane,
        &plans.profile,
        &batches.profile,
        &mut faults.profile,
        failure,
    ));
    let directory = materialize!(retain_node_scene_directory(
        &mut lane,
        plans.directory,
        NodeSceneDirectoryRefs {
            nodes,
            references,
            transforms,
        },
        SceneDomainDirectoryRefs {
            spatial: spatial.directory(),
            appearance: appearance.directory(),
            geometry: geometry.directory(),
            deformation: deformation.directory(),
            animation: animation.directory(),
            profile: profile.directory(),
        },
        plans,
    ));
    let draft = NodeSceneDraft { brand, directory };
    let output = NodeSceneActivationOutput {
        draft,
        refs: NodeSceneMaterializedRefs {
            directory,
            nodes,
            references,
            transforms,
            node_names,
            spatial,
            appearance,
            geometry,
            deformation,
            animation,
            profile,
        },
        evidence_counts: plans.evidence_counts(),
        validation_scratch: *validation_scratch,
    };
    lane.complete(directory.into_store(), output)
}

/// Runs one core execution payload with the caller's already policy-bound scoped outcome owner.
///
/// The interface accepts no raw operation, store key, account, ledger, or proof capability. Its
/// generic vocabulary is preserved through rejection, central validation, and pending joint
/// publication so the caller's sole session ledger remains the only ledger involved.
fn activate_node_scene_scoped<'input, O, PP, U, L>(
    activation: BoundNodeSceneActivation<'input, O, PP, U, L>,
    validation_progress: CanonicalValidationProgress<PP, U>,
    final_progress: FinalProgress<PP, U>,
) -> Result<NodeSceneDraftBuild<O, PP, U>, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    drive_node_scene_activation_deferred(
        activation,
        validation_progress,
        final_progress,
        NodeSceneBuildFaults::none(),
    )?
    .prepare_immediately()
}

fn activate_node_scene_scoped_deferred<'input, O, PP, U, L>(
    activation: BoundNodeSceneActivation<'input, O, PP, U, L>,
    validation_progress: CanonicalValidationProgress<PP, U>,
    final_progress: FinalProgress<PP, U>,
) -> Result<DeferredNodeSceneDraftBuild<O, PP, U, L>, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    drive_node_scene_activation_deferred(
        activation,
        validation_progress,
        final_progress,
        NodeSceneBuildFaults::none(),
    )
}

fn drive_node_scene_activation<'input, O, PP, U, L>(
    activation: BoundNodeSceneActivation<'input, O, PP, U, L>,
    validation_progress: CanonicalValidationProgress<PP, U>,
    final_progress: FinalProgress<PP, U>,
    faults: NodeSceneBuildFaults,
) -> Result<NodeSceneDraftBuild<O, PP, U>, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    drive_node_scene_activation_deferred(activation, validation_progress, final_progress, faults)?
        .prepare_immediately()
}

fn drive_node_scene_activation_deferred<'input, O, PP, U, L>(
    activation: BoundNodeSceneActivation<'input, O, PP, U, L>,
    validation_progress: CanonicalValidationProgress<PP, U>,
    final_progress: FinalProgress<PP, U>,
    faults: NodeSceneBuildFaults,
) -> Result<DeferredNodeSceneDraftBuild<O, PP, U, L>, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    #[cfg(test)]
    let panic_central_validator = faults.panic_central_validator;
    let BoundNodeSceneActivation { owner, execution } = activation;
    let NodeSceneActivationExecution {
        brand,
        input,
        validation_limits,
        failure_evidence,
    } = execution;
    let evidence_plan = failure_evidence.records;
    let sealed_counts = Cell::new(SceneEvidenceCounts::authoring_rejection());
    let sealed_counts_ref = &sealed_counts;

    let outcome = owner.plan_and_activate_scoped(move |mut planning| {
        let preflight = match planning.begin_semantic_work() {
            Ok(mut semantic) => {
                let validation = {
                    let mut control = ScopedAuthoringControl::new(&mut semantic);
                    validate_and_seal_node_scene_plan(
                        input,
                        validation_limits,
                        failure_evidence,
                        &mut control,
                    )
                };
                match validation {
                    Ok(plan) => match semantic.finish_exact() {
                        Ok(completion) => Ok((plan, completion)),
                        Err(_cause) => {
                            drop(plan);
                            Err(NodeSceneConstructionRejection::<StoreRuntimeError>::Materialization(
                                StoreRuntimeError::ConstructionInvariantUnverified,
                            ))
                        }
                    },
                    Err(rejection) => {
                        let _ = semantic.finish_rejected();
                        Err(rejection)
                    }
                }
            }
            Err(_cause) => {
                Err(NodeSceneConstructionRejection::<StoreRuntimeError>::Materialization(
                    StoreRuntimeError::ConstructionInvariantUnverified,
                ))
            }
        };
        let (plan, completion) = match preflight {
            Ok(ready) => ready,
            Err(rejection) => {
                let stable_code = rejection.stable_code();
                return planning.reject(rejection, stable_code);
            }
        };
        #[cfg(test)]
        if matches!(faults.planning, Some(NodeScenePlanningFault::SealedPlan)) {
            drop(plan);
            let rejection = NodeSceneConstructionRejection::<StoreRuntimeError>::Planning(
                StorePlanError::InvalidBlockTable,
            );
            let stable_code = rejection.stable_code();
            return planning.reject(rejection, stable_code);
        }
        sealed_counts_ref.set(plan.execution.materialization.evidence_counts());
        let (execution, success_requirements) = plan.into_seal_parts();
        #[cfg(test)]
        if matches!(faults.planning, Some(NodeScenePlanningFault::Publication)) {
            drop(execution);
            let rejection = NodeSceneConstructionRejection::<StoreRuntimeError>::Planning(
                StorePlanError::InvalidBlockTable,
            );
            let stable_code = rejection.stable_code();
            return planning.reject(rejection, stable_code);
        }
        let permit = match planning.seal_activation(success_requirements, completion) {
            CanonicalActivationSeal::Sealed(permit) => permit,
            CanonicalActivationSeal::Unsealed(failure) => {
                drop(execution);
                return failure.finish_after_plan_drop();
            }
        };
        permit.activate(execution, move |lane, execution| {
            materialize_node_scene(lane, execution, brand, faults)
        })
    })?;

    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(rejected) => {
            let authoring_rejected = rejected.cause().is_authoring_rejection();
            let control_cause = rejected.cause().control_cause().cloned();
            return Ok(DeferredNodeSceneDraftBuild::Failure {
                authoring_rejected,
                draft_built: false,
                control_cause,
                owner: PendingNodeSceneFailure::ActivationRejected {
                    owner: rejected,
                    evidence_plan,
                    evidence_counts: sealed_counts.get(),
                    final_progress,
                },
            });
        }
        CanonicalActivationOutcome::NeverActivated(never_activated) => {
            let authoring_rejected = never_activated.cause().is_authoring_rejection();
            let control_cause = never_activated.cause().control_cause().cloned();
            return Ok(DeferredNodeSceneDraftBuild::Failure {
                authoring_rejected,
                draft_built: false,
                control_cause,
                owner: PendingNodeSceneFailure::NeverActivated {
                    owner: never_activated,
                    evidence_plan,
                    evidence_counts: sealed_counts.get(),
                    final_progress,
                },
            });
        }
        CanonicalActivationOutcome::Fallback(fallback) => {
            return Ok(DeferredNodeSceneDraftBuild::Failure {
                authoring_rejected: false,
                draft_built: false,
                control_cause: None,
                owner: PendingNodeSceneFailure::PreparedFallback(fallback),
            });
        }
    };

    let central_counts = activated.output().evidence_counts;
    let outcome = activated.validate_centrally(
        validation_progress,
        |lane, output| match lane.try_retain_records(
            evidence_plan,
            NodeSceneOutcomeEvidenceSchema,
            |writer| {
                writer.write_row(|row| write_node_scene_evidence_row(row, output.evidence_counts))
            },
        ) {
            Ok(evidence) => lane.complete(evidence),
            Err(_) => lane.abort(),
        },
        |planning, output| {
            #[cfg(test)]
            if panic_central_validator {
                panic!("injected clean central validator panic");
            }
            planning.validate_with(
                |checkpoint| {
                    ScopedNodeSceneValidationScratchOwner::try_allocate(
                        checkpoint,
                        output.validation_scratch,
                    )
                },
                |session, directory, evidence, scratch| {
                    let initialized = scratch.initialize(session, output.validation_scratch)?;
                    let proof = if initialized && output.draft.brand == brand {
                        let mut scratch = scratch.metered_view();
                        validate_node_scene_canonical_graph_metered(
                            session,
                            directory,
                            evidence,
                            output.evidence_counts,
                            output.refs,
                            output.validation_scratch,
                            &mut scratch,
                        )?
                    } else {
                        None
                    };
                    Ok(match proof {
                        Some(proof) => CanonicalValidationDecision::accept(proof),
                        None => CanonicalValidationDecision::reject(
                            NodeSceneConstructionRejection::<StoreRuntimeError>::Materialization(
                                StoreRuntimeError::ConstructionInvariantUnverified,
                            ),
                            NODE_SCENE_BUILD_INVARIANT_CODE,
                        ),
                    })
                },
            )
        },
    )?;
    match outcome {
        CanonicalCentralValidationOutcome::Candidate(candidate) => {
            let draft = candidate.output().draft;
            Ok(DeferredNodeSceneDraftBuild::Draft {
                draft,
                owner: PendingNodeSceneDraft {
                    candidate,
                    final_progress,
                },
            })
        }
        CanonicalCentralValidationOutcome::Rejected(rejected) => {
            let authoring_rejected = rejected.cause().is_authoring_rejection();
            Ok(DeferredNodeSceneDraftBuild::Failure {
                authoring_rejected,
                draft_built: true,
                control_cause: None,
                owner: PendingNodeSceneFailure::CentralRejected {
                    owner: rejected,
                    evidence_counts: central_counts,
                    final_progress,
                },
            })
        }
        CanonicalCentralValidationOutcome::Interrupted(interrupted) => {
            let control_cause = interrupted.cause().control_cause().cloned();
            Ok(DeferredNodeSceneDraftBuild::Failure {
                authoring_rejected: false,
                draft_built: true,
                control_cause,
                owner: PendingNodeSceneFailure::CentralInterrupted {
                    owner: interrupted,
                    evidence_counts: central_counts,
                    final_progress,
                },
            })
        }
        CanonicalCentralValidationOutcome::Fallback(fallback) => {
            Ok(DeferredNodeSceneDraftBuild::Failure {
                authoring_rejected: false,
                draft_built: true,
                control_cause: None,
                owner: PendingNodeSceneFailure::PreparedFallback(fallback),
            })
        }
    }
}

pub(super) fn write_node_scene_evidence_row(
    row: &mut OutcomeEvidenceRecordRowWriter<'_>,
    counts: SceneEvidenceCounts,
) -> Result<(), StoreBuildError> {
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
}

fn retain_node_scene_transforms<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: SequencePlan<f32>,
    batch: &ValidatedNodeBatch<'_, R>,
    plans: &NodeSceneMaterializationPlans,
    faults: &mut NodeSceneBuildFaults,
) -> Result<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let nodes = batch.input().nodes();
    let failure = plans.build_invariant;
    lane.try_retain_sequence(plan, NodeSceneAuthoredTransformSchema, |control| {
        for &local in batch.canonical_order() {
            let authored = nodes.get(local).ok_or(failure)?.authored_transform();
            if let Some(matrix) = authored.authored_matrix() {
                for mut column in matrix.cols {
                    column[0] = faults.transform_scalar(column[0]);
                    control.extend_from_slice(&column)?;
                }
            } else if let Some(trs) = authored.authored_trs() {
                let translation = trs.translation();
                let rotation = trs.rotation();
                let scale = trs.scale();
                control.extend_from_slice(&[
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
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_node_scene_names<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: Utf8Plan,
    batch: &ValidatedNodeBatch<'_, R>,
    plans: &NodeSceneMaterializationPlans,
) -> Result<CanonicalBlockRef<NodeSceneNodeNameSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let nodes = batch.input().nodes();
    let failure = plans.build_invariant;
    lane.try_retain_utf8(plan, NodeSceneNodeNameSchema, |control| {
        let mut written = 0_u32;
        for &local in batch.canonical_order() {
            let authored = nodes.get(local).ok_or(failure)?;
            let Presence::Present(name) = authored.name() else {
                continue;
            };
            control.write_str(name)?;
            written = written
                .checked_add(u32::try_from(name.len()).map_err(|_| failure)?)
                .ok_or(failure)?;
        }
        if written != plans.node_name_byte_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_node_scene_references<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    batch: &ValidatedNodeBatch<'_, R>,
    plans: &NodeSceneMaterializationPlans,
) -> Result<CanonicalBlockRef<NodeSceneChildReferenceTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let mapping = batch.canonical_index_by_local();
    let failure = plans.build_invariant;
    lane.try_retain_records(plan, NodeSceneChildReferenceTableSchema, |control| {
        for (source_row, &local) in batch.canonical_order().iter().enumerate() {
            let reference = u32::try_from(source_row).map_err(|_| failure)?;
            let child = *mapping.get(local).ok_or(failure)?;
            control.write_row(|row| {
                row.write_scalar(reference)?;
                row.write_scalar(child)
            })?;
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[derive(Clone, Copy)]
struct RetainedNodePayloadRefs {
    transforms: Option<CanonicalBlockRef<NodeSceneAuthoredTransformSchema>>,
    names: Option<CanonicalBlockRef<NodeSceneNodeNameSchema>>,
    spatial: RetainedSpatialRefs,
}

fn retain_node_scene_nodes<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    payloads: RetainedNodePayloadRefs,
    batch: &ValidatedNodeBatch<'_, R>,
    plans: &NodeSceneMaterializationPlans,
) -> Result<CanonicalBlockRef<NodeSceneNodeTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let RetainedNodePayloadRefs {
        transforms,
        names,
        spatial,
    } = payloads;
    let authored_nodes = batch.input().nodes();
    let mapping = batch.canonical_index_by_local();
    let failure = plans.build_invariant;
    lane.try_retain_records(plan, NodeSceneNodeTableSchema, |control| {
        control.write_row(|row| {
            row.write_scalar(0_u32)?;
            row.write_scalar(PRESENCE_PRESENT)?;
            row.write_scalar(0_u32)?;
            row.write_scalar(plans.source_root_count)?;
            row.write_scalar(NODE_SCENE_TRANSFORM_SYNTHETIC)?;
            row.write_scalar(0_u32)?;
            row.write_scalar(0_u32)?;
            row.write_scalar(0_u8)?;
            row.write_scalar(PRESENCE_ABSENT)?;
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
        let mut name_offset = 0_u32;
        let mut child_reference_offset = plans.source_root_count;
        for &local in batch.canonical_order() {
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
            let (name_presence, name_count) = match authored_node.name() {
                Presence::Absent => (PRESENCE_ABSENT, 0_u32),
                Presence::Present(name) => (
                    PRESENCE_PRESENT,
                    u32::try_from(name.len()).map_err(|_| failure)?,
                ),
            };
            control.write_row(|row| {
                row.write_scalar(node)?;
                row.write_scalar(PRESENCE_PRESENT)?;
                row.write_scalar(child_reference_offset)?;
                row.write_scalar(child_count)?;
                row.write_scalar(kind)?;
                row.write_scalar(transform_offset)?;
                row.write_scalar(transform_length)?;
                row.write_sequence_ref(transforms.into_store())?;
                row.write_scalar(name_presence)?;
                row.write_scalar(name_offset)?;
                row.write_scalar(name_count)?;
                if name_count == 0 {
                    row.write_scalar(0_u8)?;
                } else {
                    row.write_utf8_ref(names.ok_or(failure)?.into_store())?;
                }
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
            name_offset = name_offset.checked_add(name_count).ok_or(failure)?;
            child_reference_offset = child_reference_offset
                .checked_add(child_count)
                .ok_or(failure)?;
        }
        if transform_offset != plans.transform_scalar_count
            || name_offset != plans.node_name_byte_count
            || child_reference_offset != plans.source_node_count
        {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_node_scene_directory<'lane, O, PP, U, L>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    refs: NodeSceneDirectoryRefs,
    domains: SceneDomainDirectoryRefs,
    plans: &NodeSceneMaterializationPlans,
) -> Result<CanonicalBlockRef<SceneDirectorySchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
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
    lane.try_retain_records(plan, SceneDirectorySchema, |control| {
        control.write_row(|row| {
            row.write_scalar(SCENE_SCHEMA_REVISION)?;
            row.write_scalar(0_u32)?;
            row.write_scalar(plans.node_count)?;
            row.write_record_ref(nodes.into_store())?;
            if plans.source_node_count == 0 {
                if references.is_some() || transforms.is_some() {
                    return Err(failure.into());
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
    })
    .map(CanonicalBlockRef::from_store)
}

fn write_node_spatial_attachment<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    local: Option<u32>,
    table: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
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
