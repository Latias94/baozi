use std::fmt;
use std::num::NonZeroU64;

#[cfg(test)]
use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalCentralValidationOutcome,
    CanonicalValidationProgress, RetainedStoreSessionOutcomeOwner,
};
use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationSessionPolicy, CanonicalFailureRequirements, CanonicalLane,
    CanonicalMaterialization, CanonicalMaterializationRole, CanonicalReservation,
    CanonicalSemanticWorkSession, CanonicalValidationDecision, CanonicalValidationPlanning,
    CanonicalValidationRole, RetainedStoreWriterRole,
};
use baozi_operation::__retained_store_integration::producer::{
    CanonicalActivationDescription, CanonicalActivationPlanSealer, CanonicalActivationRequest,
    CanonicalSealedActivationPlan, CanonicalSuccessRequirements, CanonicalTableShape,
};
#[cfg(test)]
use baozi_operation::FinalProgress;
use baozi_operation::{
    CapacityPlan, OperationOwner, OutcomeEvidenceRecordRowWriter, ProgressPhase, ProgressUnit,
    RecordBlock, RecordPlan, RetainedStoreBatchLimits, SequencePlan, StoreBuildError,
    StoreDimensionKey, StorePhaseKey, StorePlanError, StoreRuntimeError, StoreWorkError,
    StoredScalar, Utf8Plan,
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
    AnimationTargetCatalog, CanonicalAuthoringControl, ProfileOwnerCatalog, SceneAuthoringBatch,
    SceneInputEnvelope, ScopedAuthoringControl, ValidatedAnimationBatch, ValidatedAppearanceBatch,
    ValidatedDeformationBatch, ValidatedGeometryBatch, ValidatedNodeBatch, ValidatedProfileBatch,
    ValidatedSpatialBatch, animation_limits_after_deformation, appearance_limits_after_nodes,
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
use crate::canonical_scene::store::schema::spatial::SpatialDirectorySchema;
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, NODE_SCENE_EVIDENCE_FIELD_COUNT, NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE,
    NodeSceneOutcomeEvidenceSchema, SCENE_DIRECTORY_FIELD_COUNT, SCENE_SCHEMA_REVISION,
    SceneDirectorySchema,
};
#[cfg(test)]
use crate::provenance::{OperationKind, OptionsDigest};
#[cfg(test)]
use crate::scene_space::SceneSpace;

#[cfg(test)]
use super::super::SceneConstructionError;
use super::super::{SceneConstructionStartError, SceneStoreKeys};
use super::animation::{AnimationMaterializationPlans, AnimationTargetTableRefs, retain_animation};
use super::appearance::{AppearanceMaterializationPlans, retain_appearance};
use super::deformation::{
    DeformationDependencyTableRefs, DeformationMaterializationPlans, retain_deformation,
};
use super::geometry::{retain_geometry, retain_nodes};
use super::profile::{ProfileMaterializationPlans, retain_profile};
use super::scratch::{
    NodeSceneCentralValidationPlan, NodeSceneValidationScratchPlan,
    ScopedNodeSceneValidationScratchOwner,
};
use crate::canonical_scene::store::typed_storage::{
    SceneTables,
    build::{GeometryStoragePlan, NodeStoragePlan},
};
mod spatial;
mod validation;
#[cfg(test)]
use super::NodeSceneBuildFaults;
use super::{
    NODE_SCENE_BUILD_INVARIANT_CODE, NodeSceneConstructionRejection, NodeSceneDraft,
    SceneEvidenceCounts,
};
#[cfg(test)]
use super::{NodeSceneDraftBuild, PendingNodeSceneDraft, PendingNodeSceneFailure};
pub(in crate::canonical_scene::store) use spatial::RetainedSpatialRefs;
use spatial::{SpatialMaterializationPlans, retain_spatial};

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

/// Core-owned, authority-free description for one scoped NodeScene activation.
///
/// The description retains the complete producer input, its private construction brand, and one
/// non-authority activation request. Neither the request nor the sealed execution can be extracted
/// before semantic preflight succeeds and returns [`NodeSceneLanePlan`]. It carries no store key,
/// account, ledger, dormant authority, or publication authority.
#[must_use = "the NodeScene activation description must complete semantic preflight"]
pub(in crate::canonical_scene::store) struct NodeSceneActivationRequest<'input> {
    execution: NodeSceneActivationExecution<'input>,
}

/// Session-stable failure envelope bound before any Scene construction is submitted.
#[must_use = "the Scene session bootstrap must bind the import session outcome owner"]
pub struct NodeSceneSessionActivationBootstrap {
    outcome_evidence_writer_limits: RetainedStoreBatchLimits,
    failure: CanonicalFailureRequirements,
    evidence: NodeSceneEvidenceRecipe,
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

    const fn role(self) -> RetainedStoreWriterRole {
        RetainedStoreWriterRole::new(
            self.phase,
            self.capacity_dimension,
            self.attribution_dimension,
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

    pub(super) const fn session_policy(self) -> CanonicalActivationSessionPolicy {
        let policy = CanonicalActivationSessionPolicy::new(
            self.canonical_writer.role(),
            self.outcome_evidence_writer.role(),
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
    activation_request: CanonicalActivationRequest,
    input: SceneAuthoringBatch<'input>,
    validation_limits: SceneValidationLimits,
    failure_evidence: NodeSceneFailureEvidencePlan,
}

/// Core-owned half of one authority-free Scene activation transaction.
///
/// The import host retains the planning owner. Core receives only the semantic-work lane for
/// preflight and later the canonical lane for materialization. The execution payload can therefore
/// neither bind an operation nor resolve an activation outcome.
#[must_use = "the Scene activation transaction must run preflight and retain its postflight recipe"]
pub struct NodeSceneActivationTransaction<'input> {
    execution: Option<NodeSceneActivationExecution<'input>>,
    brand: super::super::ConstructionBrand,
    failure_evidence: NodeSceneFailureEvidencePlan,
    sealed_counts: SceneEvidenceCounts,
}

/// Opaque metered plan consumed only by a canonical materialization lane.
#[must_use = "the sealed Scene plan must bind its success requirements and materialize once"]
pub struct NodeSceneLanePlan<'input, 'scope, O>
where
    O: OperationOwner,
{
    brand: super::super::ConstructionBrand,
    activation_request: CanonicalActivationRequest,
    sealed: ScopedNodeScenePlan<'input, 'scope, O>,
    success_requirements: CanonicalSuccessRequirements,
}

/// Exact execution payload paired with its success facts by [`NodeSceneLanePlan`].
#[must_use = "the Scene execution plan must materialize through its bound activation permit"]
pub struct NodeSceneLaneExecutionPlan<'input, 'scope, O>
where
    O: OperationOwner,
{
    brand: super::super::ConstructionBrand,
    sealed: ScopedNodeScenePlan<'input, 'scope, O>,
}

/// The complete execution plan stays at one admitted address throughout activation. Keep the
/// allocation before its reservation so plan storage is destroyed before its charge is released.
struct ScopedNodeScenePlan<'input, 'scope, O: OperationOwner> {
    storage: Vec<NodeSceneExecutionPlan<'input, CanonicalReservation<'scope, O>>>,
    _reservation: CanonicalReservation<'scope, O>,
}

/// Opaque Scene descriptor and validation recipe emitted by lane-only materialization.
pub struct NodeSceneLaneOutput(NodeSceneActivationOutput);

/// Opaque proof minted only by exact central validation of a lane output.
#[derive(Clone, Copy)]
pub struct NodeSceneLaneValidationProof(super::NodeSceneValidationProof);

impl NodeSceneLaneValidationProof {
    pub(super) const fn from_inner(proof: super::NodeSceneValidationProof) -> Self {
        Self(proof)
    }

    pub(in crate::canonical_scene::store) const fn into_inner(
        self,
    ) -> super::NodeSceneValidationProof {
        self.0
    }
}

/// Opaque semantic or physical rejection returned across the import-owned activation rail.
#[derive(Debug)]
pub struct NodeSceneLaneRejection(NodeSceneConstructionRejection<StoreRuntimeError>);

impl NodeSceneLaneRejection {
    /// Creates the fail-closed rejection used when the host cannot resolve the scoped protocol.
    pub fn construction_invariant() -> Self {
        Self::from(NodeSceneConstructionRejection::Materialization(
            StoreRuntimeError::ConstructionInvariantUnverified,
        ))
    }

    pub const fn stable_code(&self) -> u32 {
        self.0.stable_code()
    }

    pub const fn is_authoring_rejection(&self) -> bool {
        !matches!(self.0, NodeSceneConstructionRejection::Materialization(_))
    }

    /// Returns producer-local semantic detail without allocating on the rejection path.
    pub fn scene_validation_error(&self) -> Option<crate::SceneValidationError> {
        validation::scene_validation_error(&self.0)
    }

    /// Separates authoring-lane interruption from a producer semantic rejection.
    pub fn control_cause(&self) -> Option<&baozi_operation::ControlCause> {
        use crate::canonical_scene::store::production::{
            AnimationAuthoringFailure, AppearanceAuthoringFailure, DeformationAuthoringFailure,
            GeometryAuthoringFailure, NodeAuthoringFailure, ProfileAuthoringFailure,
            SpatialAuthoringFailure,
        };

        let runtime = match &self.0 {
            NodeSceneConstructionRejection::SpatialAuthoring(SpatialAuthoringFailure::Control(
                cause,
            ))
            | NodeSceneConstructionRejection::NodeAuthoring(NodeAuthoringFailure::Control(cause))
            | NodeSceneConstructionRejection::AppearanceAuthoring(
                AppearanceAuthoringFailure::Control(cause),
            )
            | NodeSceneConstructionRejection::GeometryAuthoring(
                GeometryAuthoringFailure::Control(cause),
            )
            | NodeSceneConstructionRejection::DeformationAuthoring(
                DeformationAuthoringFailure::Control(cause),
            )
            | NodeSceneConstructionRejection::AnimationAuthoring(
                AnimationAuthoringFailure::Control(cause),
            )
            | NodeSceneConstructionRejection::ProfileAuthoring(ProfileAuthoringFailure::Control(
                cause,
            ))
            | NodeSceneConstructionRejection::Materialization(cause) => cause,
            _ => return None,
        };
        match runtime {
            StoreRuntimeError::Control(cause) => Some(cause),
            _ => None,
        }
    }

    pub(super) fn into_inner(self) -> NodeSceneConstructionRejection<StoreRuntimeError> {
        self.0
    }
}

impl fmt::Display for NodeSceneLaneRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for NodeSceneLaneRejection {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl From<NodeSceneConstructionRejection<StoreRuntimeError>> for NodeSceneLaneRejection {
    fn from(rejection: NodeSceneConstructionRejection<StoreRuntimeError>) -> Self {
        Self(rejection)
    }
}

/// Lifetime-free core recipe retained after the scoped preflight transaction returns.
#[must_use = "postflight evidence and validation must resolve the activation outcome"]
pub struct NodeSceneActivationPostflight {
    brand: super::super::ConstructionBrand,
    failure_evidence: NodeSceneFailureEvidencePlan,
    evidence_counts: SceneEvidenceCounts,
}

/// Exact evidence plan and row recipe derived from the activation transaction.
#[derive(Clone, Copy)]
pub struct NodeSceneEvidenceRecipe {
    records: RecordPlan,
    counts: SceneEvidenceCounts,
}

impl<'input> NodeSceneActivationRequest<'input> {
    pub(in crate::canonical_scene::store) fn try_new(
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
    ) -> Result<Self, SceneConstructionStartError> {
        let envelope = SceneInputEnvelope::try_new(input, validation_limits)?;
        let _ = envelope;
        let failure_evidence = NodeSceneFailureEvidencePlan::try_new()?;
        let (brand, activation_request) =
            super::super::ConstructionBrand::reserve_with_activation_request()?;
        Ok(Self {
            execution: NodeSceneActivationExecution {
                brand,
                activation_request,
                input,
                validation_limits,
                failure_evidence,
            },
        })
    }

    pub fn into_transaction(self) -> NodeSceneActivationTransaction<'input> {
        let Self { execution } = self;
        let brand = execution.brand;
        let failure_evidence = execution.failure_evidence;
        NodeSceneActivationTransaction {
            execution: Some(execution),
            brand,
            failure_evidence,
            sealed_counts: SceneEvidenceCounts::authoring_rejection(),
        }
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn activate<O, PP, U>(
        self,
        owner: RetainedStoreSessionOutcomeOwner<O, PP, U>,
        validation_progress: CanonicalValidationProgress<PP, U>,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<NodeSceneDraftBuild<O, PP, U>, SceneConstructionError>
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        drive_node_scene_test_activation(
            owner,
            self.into_transaction(),
            validation_progress,
            final_progress,
            NodeSceneBuildFaults::none(),
        )
    }
}

impl<'input> NodeSceneActivationTransaction<'input> {
    /// Runs semantic validation and physical planning without receiving operation ownership.
    ///
    /// The caller remains responsible for resolving the semantic session with `finish_exact` or
    /// `finish_rejected`, and for sealing or rejecting its own planning transaction.
    pub fn preflight<'scope, O, PP, U, L>(
        &mut self,
        semantic: &mut CanonicalSemanticWorkSession<'_, 'scope, O, PP, U, L>,
    ) -> Result<NodeSceneLanePlan<'input, 'scope, O>, NodeSceneLaneRejection>
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        let execution = self.execution.take().ok_or_else(|| {
            NodeSceneLaneRejection::from(NodeSceneConstructionRejection::Materialization(
                StoreRuntimeError::ConstructionInvariantUnverified,
            ))
        })?;
        let NodeSceneActivationExecution {
            brand,
            activation_request,
            input,
            validation_limits,
            failure_evidence,
        } = execution;
        let capacity = CapacityPlan::for_type::<
            NodeSceneExecutionPlan<'input, CanonicalReservation<'scope, O>>,
        >(0, 0, 1)
        .map_err(|_| {
            NodeSceneLaneRejection::from(NodeSceneConstructionRejection::Planning(
                StorePlanError::SizeOverflow,
            ))
        })?;
        let mut reservation = semantic
            .reserve(capacity.envelope_delta_bytes().map_err(|_| {
                NodeSceneLaneRejection::from(NodeSceneConstructionRejection::Planning(
                    StorePlanError::SizeOverflow,
                ))
            })?)
            .map_err(NodeSceneConstructionRejection::Materialization)?;
        // Declare storage after the reservation as well: an early return must release the
        // allocation before making its headroom available again.
        let mut storage = Vec::new();
        storage
            .try_reserve_exact(capacity.allocator_additional())
            .map_err(|_| {
                NodeSceneLaneRejection::from(NodeSceneConstructionRejection::Materialization(
                    StoreRuntimeError::BatchAllocationFailed,
                ))
            })?;
        let bytes = capacity
            .finish(storage.len(), storage.capacity())
            .map_err(|_| {
                NodeSceneLaneRejection::from(NodeSceneConstructionRejection::Materialization(
                    StoreRuntimeError::ConstructionInvariantUnverified,
                ))
            })?;
        semantic
            .resize(&mut reservation, bytes)
            .map_err(NodeSceneConstructionRejection::Materialization)?;
        let sealed = {
            let mut control = ScopedAuthoringControl::new(semantic);
            validate_and_seal_node_scene_plan(
                input,
                validation_limits,
                failure_evidence,
                brand,
                &mut control,
            )
        }
        .map_err(NodeSceneLaneRejection::from)?;
        let NodeSceneSealedPlan {
            execution,
            success_requirements,
        } = sealed;
        self.sealed_counts = execution.materialization.evidence_counts();
        storage.push(execution);
        Ok(NodeSceneLanePlan {
            brand,
            activation_request,
            sealed: ScopedNodeScenePlan {
                storage,
                _reservation: reservation,
            },
            success_requirements,
        })
    }

    /// Drops any unused execution payload and returns the lifetime-free evidence/validation recipe.
    pub fn finish(mut self) -> NodeSceneActivationPostflight {
        self.execution = None;
        NodeSceneActivationPostflight {
            brand: self.brand,
            failure_evidence: self.failure_evidence,
            evidence_counts: self.sealed_counts,
        }
    }
}

impl<'input, 'scope, O> NodeSceneLanePlan<'input, 'scope, O>
where
    O: OperationOwner,
{
    fn into_activation_parts(
        self,
    ) -> (
        CanonicalActivationRequest,
        CanonicalSuccessRequirements,
        NodeSceneLaneExecutionPlan<'input, 'scope, O>,
    ) {
        let Self {
            brand,
            activation_request,
            sealed,
            success_requirements,
        } = self;
        (
            activation_request,
            success_requirements,
            NodeSceneLaneExecutionPlan { brand, sealed },
        )
    }
}

impl<'input, 'scope, O> CanonicalActivationDescription for NodeSceneLanePlan<'input, 'scope, O>
where
    O: OperationOwner,
{
    type Execution = NodeSceneLaneExecutionPlan<'input, 'scope, O>;

    fn seal_activation<'activation>(
        self,
        sealer: CanonicalActivationPlanSealer<'activation>,
    ) -> CanonicalSealedActivationPlan<'activation, Self::Execution> {
        let (activation_request, requirements, execution) = self.into_activation_parts();
        sealer.seal(activation_request, requirements, execution)
    }
}

impl<'input, 'scope, O> NodeSceneLaneExecutionPlan<'input, 'scope, O>
where
    O: OperationOwner,
{
    /// Materializes the sealed plan through the only authority accepted by this core seam.
    pub fn materialize<'lane, PP, U, L>(
        &self,
        lane: CanonicalLane<'lane, O, PP, U, L>,
    ) -> CanonicalMaterialization<
        'lane,
        NodeSceneLaneOutput,
        NodeSceneLaneRejection,
        RecordBlock,
        SceneDirectorySchema,
    >
    where
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        self.materialize_mapped(
            lane,
            #[cfg(test)]
            NodeSceneBuildFaults::none(),
        )
    }

    fn materialize_mapped<'lane, PP, U, L>(
        &self,
        lane: CanonicalLane<'lane, O, PP, U, L>,
        #[cfg(test)] faults: NodeSceneBuildFaults,
    ) -> CanonicalMaterialization<
        'lane,
        NodeSceneLaneOutput,
        NodeSceneLaneRejection,
        RecordBlock,
        SceneDirectorySchema,
    >
    where
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        materialize_node_scene_mapped(
            lane,
            &self.sealed.storage[0],
            self.brand,
            #[cfg(test)]
            faults,
            NodeSceneLaneOutput,
            NodeSceneLaneRejection,
        )
    }

    #[cfg(test)]
    fn materialize_with_faults<'lane, PP, U, L>(
        &self,
        lane: CanonicalLane<'lane, O, PP, U, L>,
        faults: NodeSceneBuildFaults,
    ) -> CanonicalMaterialization<
        'lane,
        NodeSceneLaneOutput,
        NodeSceneLaneRejection,
        RecordBlock,
        SceneDirectorySchema,
    >
    where
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        self.materialize_mapped(lane, faults)
    }
}

impl NodeSceneLaneOutput {
    pub(super) const fn from_inner(output: NodeSceneActivationOutput) -> Self {
        Self(output)
    }

    pub(in crate::canonical_scene::store) fn into_tables(self) -> SceneTables {
        self.0.tables
    }

    pub(in crate::canonical_scene::store) const fn inner(&self) -> &NodeSceneActivationOutput {
        &self.0
    }

    pub(in crate::canonical_scene::store) const fn node_scene_draft(&self) -> NodeSceneDraft {
        self.0.draft
    }
}

impl NodeSceneEvidenceRecipe {
    pub(super) const fn from_counts(records: RecordPlan, counts: SceneEvidenceCounts) -> Self {
        Self { records, counts }
    }

    /// Returns the exact retained evidence-table plan for this recipe.
    pub const fn record_plan(&self) -> RecordPlan {
        self.records
    }

    /// Writes the sole NodeScene evidence row without receiving storage ownership.
    pub fn write_row(
        &self,
        row: &mut OutcomeEvidenceRecordRowWriter<'_>,
    ) -> Result<(), StoreBuildError> {
        write_node_scene_evidence_row(row, self.counts)
    }

    pub(super) const fn counts(&self) -> SceneEvidenceCounts {
        self.counts
    }
}

impl NodeSceneActivationPostflight {
    pub(super) const fn from_parts(
        brand: super::super::ConstructionBrand,
        failure_evidence: NodeSceneFailureEvidencePlan,
        evidence_counts: SceneEvidenceCounts,
    ) -> Self {
        Self {
            brand,
            failure_evidence,
            evidence_counts,
        }
    }

    /// Returns the failure recipe for a preflight or materialization rejection.
    pub const fn failure_evidence(&self) -> NodeSceneEvidenceRecipe {
        NodeSceneEvidenceRecipe {
            records: self.failure_evidence.records,
            counts: self.evidence_counts,
        }
    }

    /// Returns the candidate recipe derived from the exact materialization output.
    pub const fn candidate_evidence(
        &self,
        output: &NodeSceneLaneOutput,
    ) -> NodeSceneEvidenceRecipe {
        NodeSceneEvidenceRecipe {
            records: self.failure_evidence.records,
            counts: output.0.evidence_counts,
        }
    }

    /// Runs core graph validation through an import-owned central-validation planning lane.
    #[allow(clippy::type_complexity)]
    pub fn validate_candidate<'session, 'storage, 'scope, 'operation, O, PP, U>(
        &self,
        planning: CanonicalValidationPlanning<
            'session,
            'storage,
            'scope,
            'operation,
            O,
            PP,
            U,
            RecordBlock,
            SceneDirectorySchema,
            RecordBlock,
            NodeSceneOutcomeEvidenceSchema,
            NodeSceneLaneValidationProof,
            NodeSceneLaneRejection,
        >,
        output: &NodeSceneLaneOutput,
    ) -> Result<(), StoreWorkError>
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        let output = &output.0;
        let validation_plan = output.central_validation;
        let scratch_plan = validation_plan.scratch_plan();
        planning.validate_with(
            |checkpoint| {
                ScopedNodeSceneValidationScratchOwner::try_allocate(checkpoint, scratch_plan)
            },
            |session, directory, evidence, scratch| {
                let initialized = scratch.initialize(session, scratch_plan)?;
                let proof = if initialized && output.draft.brand == self.brand {
                    let mut scratch = scratch.metered_view();
                    validate_node_scene_canonical_graph_metered(
                        session,
                        directory,
                        evidence,
                        output.evidence_counts,
                        output.refs,
                        &output.tables,
                        validation_plan,
                        &mut scratch,
                    )?
                } else {
                    None
                };
                Ok(match proof {
                    Some(proof) => {
                        CanonicalValidationDecision::accept(NodeSceneLaneValidationProof(proof))
                    }
                    None => CanonicalValidationDecision::reject(
                        NodeSceneLaneRejection::from(
                            NodeSceneConstructionRejection::Materialization(
                                StoreRuntimeError::ConstructionInvariantUnverified,
                            ),
                        ),
                        NODE_SCENE_BUILD_INVARIANT_CODE,
                    ),
                })
            },
        )
    }
}

impl NodeSceneSessionActivationBootstrap {
    /// Builds the exact session-bound failure plan before any Scene construction is submitted.
    pub fn try_new() -> Result<Self, StorePlanError> {
        let failure_evidence = NodeSceneFailureEvidencePlan::try_new()?;
        Ok(Self {
            outcome_evidence_writer_limits: RetainedStoreBatchLimits::new(2, 4)?,
            failure: failure_evidence.requirements()?,
            evidence: NodeSceneEvidenceRecipe {
                records: failure_evidence.records,
                counts: SceneEvidenceCounts::authoring_rejection(),
            },
        })
    }

    pub fn into_parts(
        self,
    ) -> (
        RetainedStoreBatchLimits,
        CanonicalFailureRequirements,
        NodeSceneEvidenceRecipe,
    ) {
        (
            self.outcome_evidence_writer_limits,
            self.failure,
            self.evidence,
        )
    }
}

fn validate_node_scene_batches<'input, C>(
    input: SceneAuthoringBatch<'input>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<SceneStorageBatches<'input, C::Reservation>, NodeSceneConstructionRejection<C::Error>>
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
    Ok(SceneStorageBatches {
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
    brand: super::super::ConstructionBrand,
    control: &mut C,
) -> Result<NodeSceneSealedPlan<'input, C::Reservation>, NodeSceneConstructionRejection<C::Error>>
where
    C: CanonicalAuthoringControl,
{
    let batches = validate_node_scene_batches(input, limits, control)?;
    NodeSceneSealedPlan::try_new(batches, failure_evidence, brand)
        .map_err(NodeSceneConstructionRejection::Planning)
}

#[derive(Clone, Copy)]
struct SceneDomainDirectoryRefs {
    spatial: CanonicalBlockRef<SpatialDirectorySchema>,
    appearance: CanonicalBlockRef<AppearanceDirectorySchema>,
    deformation: CanonicalBlockRef<DeformationDirectorySchema>,
    animation: CanonicalBlockRef<AnimationDirectorySchema>,
}

struct SceneStorageBatches<'input, R> {
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
    batches: SceneStorageBatches<'input, R>,
    materialization: NodeSceneMaterializationPlans,
    central_validation: NodeSceneCentralValidationPlan,
}

#[derive(Clone, Copy)]
pub(super) struct NodeSceneFailureEvidencePlan {
    pub(super) records: RecordPlan,
}

impl NodeSceneFailureEvidencePlan {
    pub(super) fn try_new() -> Result<Self, StorePlanError> {
        Ok(Self {
            records: RecordPlan::new(1, NODE_SCENE_EVIDENCE_FIELD_COUNT)?,
        })
    }

    pub(super) fn requirements(self) -> Result<CanonicalFailureRequirements, StorePlanError> {
        Ok(CanonicalFailureRequirements::new(CanonicalTableShape::new(
            1, 1,
        )?))
    }

    pub(super) fn requirements_for(
        self,
        brand: super::super::ConstructionBrand,
    ) -> Result<CanonicalFailureRequirements, StorePlanError> {
        self.requirements().map(|requirements| {
            requirements.with_request_identity(brand.activation_request_identity())
        })
    }

    pub(super) fn validation_work_units(self) -> Result<u64, StorePlanError> {
        record_validation_work(self.records)
    }
}

pub(in crate::canonical_scene::store) fn maximum_node_scene_structural_work(
    canonical_blocks: usize,
) -> Option<u64> {
    let success = CanonicalSuccessRequirements::new(
        CanonicalTableShape::new(canonical_blocks, canonical_blocks).ok()?,
        4,
        canonical_blocks,
        NonZeroU64::MIN,
    )
    .ok()?;
    let failure = NodeSceneFailureEvidencePlan::try_new()
        .ok()?
        .requirements()
        .ok()?;
    success.structural_work_requirement(&failure).ok()
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneMaterializedRefs {
    pub(in crate::canonical_scene::store) directory: CanonicalBlockRef<SceneDirectorySchema>,
    pub(in crate::canonical_scene::store) spatial: RetainedSpatialRefs,
    pub(in crate::canonical_scene::store) appearance: super::appearance::RetainedAppearanceRefs,
    pub(in crate::canonical_scene::store) deformation: super::deformation::RetainedDeformationRefs,
    pub(in crate::canonical_scene::store) animation: super::animation::RetainedAnimationRefs,
}

pub(in crate::canonical_scene::store) struct NodeSceneActivationOutput {
    pub(in crate::canonical_scene::store) tables:
        crate::canonical_scene::store::typed_storage::SceneTables,
    pub(in crate::canonical_scene::store) draft: NodeSceneDraft,
    pub(in crate::canonical_scene::store) refs: NodeSceneMaterializedRefs,
    pub(in crate::canonical_scene::store) evidence_counts: SceneEvidenceCounts,
    pub(in crate::canonical_scene::store) central_validation: NodeSceneCentralValidationPlan,
}

struct NodeSceneMaterializationPlans {
    nodes: NodeStoragePlan,
    spatial: SpatialMaterializationPlans,
    appearance: AppearanceMaterializationPlans,
    geometry: GeometryStoragePlan,
    deformation: DeformationMaterializationPlans,
    animation: AnimationMaterializationPlans,
    profile: ProfileMaterializationPlans,
    directory: RecordPlan,
    canonical_block_count: u32,
    node_count: u32,
    source_root_count: u32,
    canonical_string_bytes: u64,
    payload_block_count: u32,
    shareable_leaf_block_count: u32,
    build_invariant: StoreBuildError,
}

impl<'input, R> NodeSceneSealedPlan<'input, R> {
    fn try_new(
        batches: SceneStorageBatches<'input, R>,
        failure_evidence: NodeSceneFailureEvidencePlan,
        brand: super::super::ConstructionBrand,
    ) -> Result<Self, StorePlanError> {
        let materialization = NodeSceneMaterializationPlans::new(&batches)?;
        let validation_scratch = NodeSceneValidationScratchPlan::try_new(
            materialization.evidence_counts(),
            batches.deformation.maximum_uniqueness_capacity(),
            materialization.deformation.influence_row_offset_count(),
            materialization.deformation.influence_count(),
            materialization.animation.metered_counts(),
            materialization.profile.metered_counts(),
            materialization.profile.maximum_provenance_depth(),
        )?;
        let central_validation = NodeSceneCentralValidationPlan::try_new(
            validation_scratch,
            materialization.validation_work_units(validation_scratch, failure_evidence)?,
        )?;
        let success_requirements =
            materialization.success_requirements(central_validation, brand)?;
        Ok(Self {
            execution: NodeSceneExecutionPlan {
                batches,
                materialization,
                central_validation,
            },
            success_requirements,
        })
    }
}

impl NodeSceneMaterializationPlans {
    fn success_requirements(
        &self,
        central_validation: NodeSceneCentralValidationPlan,
        brand: super::super::ConstructionBrand,
    ) -> Result<CanonicalSuccessRequirements, StorePlanError> {
        let canonical_block_count = usize::try_from(self.canonical_block_count)
            .map_err(|_| StorePlanError::SizeOverflow)?;
        CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(canonical_block_count, canonical_block_count)?,
            4,
            canonical_block_count,
            central_validation.work(),
        )
        .map(|requirements| {
            requirements
                .with_request_identity(brand.activation_request_identity())
                .with_semantic_bytes(self.canonical_string_bytes)
        })
    }

    fn validation_work_units(
        &self,
        validation_scratch: NodeSceneValidationScratchPlan,
        failure_evidence: NodeSceneFailureEvidencePlan,
    ) -> Result<u64, StorePlanError> {
        [
            failure_evidence.validation_work_units()?,
            self.nodes.validation_work()?,
            self.spatial.validation_work_units()?,
            self.appearance.validation_work_units()?,
            self.geometry.validation_work()?,
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
        let profile = self.profile.metered_counts();
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
            profile_segment_count: profile.segment_count,
            profile_resource_count: profile.resource_count,
            profile_source_count: profile.source_count,
            profile_provenance_count: profile.provenance_count,
            profile_operation_count: profile.operation_count,
            profile_parent_count: profile.parent_count,
            profile_owner_count: profile.owner_count,
            profile_attachment_count: profile.attachment_count,
            profile_metadata_entry_count: profile.metadata_entry_count,
            shareable_leaf_block_count: self.shareable_leaf_block_count,
        }
    }

    fn new<R>(batches: &SceneStorageBatches<'_, R>) -> Result<Self, StorePlanError> {
        let spatial_batch = &batches.spatial;
        let nodes_batch = &batches.nodes;
        let appearance_batch = &batches.appearance;
        let geometry_batch = &batches.geometry;
        let deformation_batch = &batches.deformation;
        let animation_batch = &batches.animations;
        let profile_batch = &batches.profile;
        let node_count = u32::try_from(nodes_batch.canonical_node_count())
            .map_err(|_| StorePlanError::SizeOverflow)?;
        let source_root_count = nodes_batch.source_root_count();
        let profile_shape = profile_batch.retention_shape();
        let canonical_string_bytes = [
            nodes_batch.node_name_byte_count(),
            u64::from(spatial_batch.tangent_basis_schema_name_bytes()),
            u64::from(spatial_batch.tangent_encoding_schema_name_bytes()),
            u64::from(appearance_batch.media_type_byte_count()),
            u64::from(appearance_batch.source_key_byte_count()),
            u64::from(appearance_batch.material_name_byte_count()),
            u64::from(geometry_batch.logical_mesh_name_byte_count()),
            u64::from(geometry_batch.attribute_semantic_name_byte_count()),
            u64::from(deformation_batch.skin_name_byte_count()),
            u64::from(deformation_batch.morph_name_byte_count()),
            u64::from(animation_batch.clip_name_byte_count()),
            u64::from(profile_shape.lineage_text_bytes()),
            u64::from(profile_shape.metadata_text_bytes()),
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
        .ok_or(StorePlanError::SizeOverflow)?;

        let nodes = NodeStoragePlan::new(nodes_batch)?;
        let spatial = SpatialMaterializationPlans::new(spatial_batch)?;
        let appearance = AppearanceMaterializationPlans::new(appearance_batch)?;
        let geometry = GeometryStoragePlan::new(geometry_batch)?;
        let deformation = DeformationMaterializationPlans::new(deformation_batch)?;
        let animation = AnimationMaterializationPlans::new(animation_batch)?;
        let profile = ProfileMaterializationPlans::new(profile_batch)?;
        let directory = RecordPlan::new(1, SCENE_DIRECTORY_FIELD_COUNT)?;
        let canonical_block_count_usize = 1_usize
            .checked_add(spatial.canonical_block_count())
            .and_then(|count| count.checked_add(appearance.canonical_block_count()))
            .and_then(|count| count.checked_add(deformation.canonical_block_count()))
            .and_then(|count| count.checked_add(animation.canonical_block_count()))
            .ok_or(StorePlanError::SizeOverflow)?;
        let canonical_block_count =
            u32::try_from(canonical_block_count_usize).map_err(|_| StorePlanError::SizeOverflow)?;
        let domain_payload_block_count = appearance
            .payload_block_count()
            .checked_add(spatial.payload_block_count())
            .and_then(|count| count.checked_add(deformation.payload_block_count()))
            .and_then(|count| count.checked_add(animation.payload_block_count()))
            .ok_or(StorePlanError::SizeOverflow)?;
        let payload_block_count = domain_payload_block_count;
        let shareable_leaf_block_count = domain_payload_block_count;
        let build_invariant = StoreBuildError::domain(NODE_SCENE_BUILD_INVARIANT_CODE)?;

        Ok(Self {
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
            source_root_count,
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

pub(super) fn node_scene_activation_roles(keys: SceneStoreKeys) -> NodeSceneActivationRoles {
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
    .with_canonical_string_dimension(keys.dimension)
}

fn materialize_node_scene<'lane, R, O, PP, U, L>(
    lane: CanonicalLane<'lane, O, PP, U, L>,
    plan: &NodeSceneExecutionPlan<'_, R>,
    brand: super::super::ConstructionBrand,
    #[cfg(test)] faults: NodeSceneBuildFaults,
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
    materialize_node_scene_mapped(
        lane,
        plan,
        brand,
        #[cfg(test)]
        faults,
        |output| output,
        |rejection| rejection,
    )
}

#[allow(clippy::too_many_arguments)]
fn materialize_node_scene_mapped<'lane, R, O, PP, U, L, T, E>(
    mut lane: CanonicalLane<'lane, O, PP, U, L>,
    plan: &NodeSceneExecutionPlan<'_, R>,
    brand: super::super::ConstructionBrand,
    #[cfg(test)] mut faults: NodeSceneBuildFaults,
    map_output: fn(NodeSceneActivationOutput) -> T,
    map_rejection: fn(NodeSceneConstructionRejection<StoreRuntimeError>) -> E,
) -> CanonicalMaterialization<'lane, T, E, RecordBlock, SceneDirectorySchema>
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
                    return lane.reject(map_rejection(rejection), stable_code);
                }
            }
        };
    }

    let NodeSceneExecutionPlan {
        batches,
        materialization: plans,
        central_validation,
    } = plan;
    let failure = plans.build_invariant;
    let spatial = materialize!(retain_spatial(
        &mut lane,
        &plans.spatial,
        &batches.spatial,
        failure,
    ));
    let nodes = materialize!(retain_nodes(
        &mut lane,
        plans.nodes,
        &batches.nodes,
        #[cfg(test)]
        &mut faults,
    ));
    let appearance = materialize!(retain_appearance(
        &mut lane,
        &plans.appearance,
        &batches.appearance,
        failure,
    ));
    let geometry = materialize!(retain_geometry(
        &mut lane,
        plans.geometry,
        &batches.geometry,
        &batches.nodes,
        #[cfg(test)]
        &mut faults,
    ));
    let deformation = materialize!(retain_deformation(
        &mut lane,
        &plans.deformation,
        &batches.deformation,
        &batches.nodes,
        materialize!(DeformationDependencyTableRefs::new(
            nodes.as_ref(),
            geometry.as_ref()
        )),
        failure,
    ));
    let animation = materialize!(retain_animation(
        &mut lane,
        &plans.animation,
        &batches.animations,
        AnimationTargetTableRefs::new(plans.node_count, deformation.mesh_instances()),
        failure,
    ));
    let profile = materialize!(retain_profile(
        &mut lane,
        &plans.profile,
        &batches.profile,
        #[cfg(test)]
        &mut faults.profile,
    ));
    let directory = materialize!(retain_node_scene_directory(
        &mut lane,
        plans.directory,
        SceneDomainDirectoryRefs {
            spatial: spatial.directory(),
            appearance: appearance.directory(),
            deformation: deformation.directory(),
            animation: animation.directory(),
        },
        plans,
    ));
    let draft = NodeSceneDraft { brand, directory };
    let output = NodeSceneActivationOutput {
        tables: SceneTables {
            nodes,
            geometry,
            profile,
        },
        draft,
        refs: NodeSceneMaterializedRefs {
            directory,
            spatial,
            appearance,
            deformation,
            animation,
        },
        evidence_counts: plans.evidence_counts(),
        central_validation: *central_validation,
    };
    lane.complete(directory.into_store(), map_output(output))
}

#[cfg(test)]
pub(super) fn activate_node_scene_with_faults_for_test<'input, O, PP, U>(
    request: NodeSceneActivationRequest<'input>,
    owner: RetainedStoreSessionOutcomeOwner<O, PP, U>,
    validation_progress: CanonicalValidationProgress<PP, U>,
    final_progress: FinalProgress<PP, U>,
    faults: NodeSceneBuildFaults,
) -> Result<NodeSceneDraftBuild<O, PP, U>, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    drive_node_scene_test_activation(
        owner,
        request.into_transaction(),
        validation_progress,
        final_progress,
        faults,
    )
}

#[cfg(test)]
fn drive_node_scene_test_activation<'input, O, PP, U, L>(
    owner: RetainedStoreSessionOutcomeOwner<O, PP, U, L>,
    mut transaction: NodeSceneActivationTransaction<'input>,
    validation_progress: CanonicalValidationProgress<PP, U>,
    final_progress: FinalProgress<PP, U>,
    faults: NodeSceneBuildFaults,
) -> Result<NodeSceneDraftBuild<O, PP, U>, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let panic_central_validator = faults.panic_central_validator;
    let outcome = owner.plan_and_activate_scoped(|mut planning| {
        let preflight = match planning.begin_semantic_work() {
            Ok(mut semantic) => match transaction.preflight(&mut semantic) {
                Ok(plan) => match semantic.finish_exact() {
                    Ok(completion) => Ok((plan, completion)),
                    Err(_cause) => {
                        drop(plan);
                        Err(NodeSceneLaneRejection::construction_invariant())
                    }
                },
                Err(rejection) => {
                    let _ = semantic.finish_rejected();
                    Err(rejection)
                }
            },
            Err(_cause) => Err(NodeSceneLaneRejection::construction_invariant()),
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
            let rejection = NodeSceneLaneRejection::from(NodeSceneConstructionRejection::<
                StoreRuntimeError,
            >::Planning(
                StorePlanError::InvalidBlockTable
            ));
            let stable_code = rejection.stable_code();
            return planning.reject(rejection, stable_code);
        }
        if matches!(faults.planning, Some(NodeScenePlanningFault::Publication)) {
            drop(plan);
            let rejection = NodeSceneLaneRejection::from(NodeSceneConstructionRejection::<
                StoreRuntimeError,
            >::Planning(
                StorePlanError::InvalidBlockTable
            ));
            let stable_code = rejection.stable_code();
            return planning.reject(rejection, stable_code);
        }
        let permit = match planning.seal_activation(plan, completion) {
            CanonicalActivationSeal::Sealed(permit) => permit,
            CanonicalActivationSeal::Unsealed(failure) => return failure.finish(),
        };
        permit.activate(move |lane, execution| execution.materialize_with_faults(lane, faults))
    })?;
    let postflight = transaction.finish();
    let failure_evidence = postflight.failure_evidence();

    let activated = match outcome {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(rejected) => {
            return prepare_node_scene_test_failure::<O, PP, U, L>(
                PendingNodeSceneFailure::ActivationRejected {
                    owner: rejected,
                    evidence_plan: failure_evidence.record_plan(),
                    evidence_counts: failure_evidence.counts(),
                    final_progress,
                },
            );
        }
        CanonicalActivationOutcome::NeverActivated(never_activated) => {
            return prepare_node_scene_test_failure::<O, PP, U, L>(
                PendingNodeSceneFailure::NeverActivated {
                    owner: never_activated,
                    evidence_plan: failure_evidence.record_plan(),
                    evidence_counts: failure_evidence.counts(),
                    final_progress,
                },
            );
        }
        CanonicalActivationOutcome::Fallback(fallback) => {
            return prepare_node_scene_test_failure::<O, PP, U, L>(
                PendingNodeSceneFailure::PreparedFallback(fallback),
            );
        }
    };

    let candidate_evidence = postflight.candidate_evidence(activated.output());
    let central_counts = candidate_evidence.counts();
    let outcome = activated.validate_centrally(
        validation_progress,
        |lane, _output| match lane.try_retain_records(
            candidate_evidence.record_plan(),
            NodeSceneOutcomeEvidenceSchema,
            |writer| writer.write_row(|row| candidate_evidence.write_row(row)),
        ) {
            Ok(evidence) => lane.complete(evidence),
            Err(_) => lane.abort(),
        },
        |planning, output| {
            if panic_central_validator {
                panic!("injected clean central validator panic");
            }
            postflight.validate_candidate(planning, output)
        },
    )?;
    match outcome {
        CanonicalCentralValidationOutcome::Candidate(candidate) => {
            let draft = candidate.output().node_scene_draft();
            Ok(NodeSceneDraftBuild::Draft {
                draft,
                owner: PendingNodeSceneDraft {
                    candidate,
                    final_progress,
                },
            })
        }
        CanonicalCentralValidationOutcome::Rejected(rejected) => {
            prepare_node_scene_test_failure::<O, PP, U, L>(
                PendingNodeSceneFailure::CentralRejected {
                    owner: rejected,
                    evidence_counts: central_counts,
                    final_progress,
                },
            )
        }
        CanonicalCentralValidationOutcome::Interrupted(interrupted) => {
            prepare_node_scene_test_failure::<O, PP, U, L>(
                PendingNodeSceneFailure::CentralInterrupted {
                    owner: interrupted,
                    evidence_counts: central_counts,
                    final_progress,
                },
            )
        }
        CanonicalCentralValidationOutcome::Fallback(fallback) => {
            prepare_node_scene_test_failure::<O, PP, U, L>(
                PendingNodeSceneFailure::PreparedFallback(fallback),
            )
        }
    }
}

#[cfg(test)]
fn prepare_node_scene_test_failure<O, PP, U, L>(
    pending: PendingNodeSceneFailure<O, PP, U, L>,
) -> Result<NodeSceneDraftBuild<O, PP, U>, SceneConstructionError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let (prepared, authoring_rejection, _control_cause) = pending.prepare()?;
    Ok(NodeSceneDraftBuild::Failure {
        authoring_rejection,
        prepared,
    })
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

fn retain_node_scene_directory<'lane, O, PP, U, L>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    domains: SceneDomainDirectoryRefs,
    plans: &NodeSceneMaterializationPlans,
) -> Result<CanonicalBlockRef<SceneDirectorySchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, SceneDirectorySchema, |control| {
        control.write_row(|row| {
            row.write_scalar(SCENE_SCHEMA_REVISION)?;
            row.write_scalar(plans.node_count)?;
            row.write_scalar(plans.source_root_count)?;
            row.write_scalar(plans.geometry.logical_mesh_count())?;
            row.write_scalar(plans.geometry.primitive_count())?;
            row.write_scalar(plans.geometry.attribute_count())?;
            row.write_scalar(plans.geometry.mesh_instance_count())?;
            row.write_record_ref(domains.spatial.into_store())?;
            row.write_record_ref(domains.appearance.into_store())?;
            row.write_record_ref(domains.deformation.into_store())?;
            row.write_record_ref(domains.animation.into_store())
        })
    })
    .map(CanonicalBlockRef::from_store)
}
