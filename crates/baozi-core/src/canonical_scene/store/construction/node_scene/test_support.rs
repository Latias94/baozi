use baozi_operation::__retained_store_integration::activation::{
    CanonicalValidationProgress, RetainedStoreSessionOutcomeOwner,
};
use baozi_operation::{OperationControl, PhysicalStorageAccount};

use crate::SceneValidationLimits;

use super::super::super::production::{SceneAuthoringBatch, SceneInputEnvelope};
use super::super::super::schema::NODE_SCENE_EVIDENCE_FIELD_COUNT;
use super::super::{
    SceneConstructionLimits, SceneConstructionPhase, SceneConstructionStartError,
    SceneConstructionUnit, SceneFailureEvidenceWork, SceneStoreOperation, final_progress,
    try_plan_scene_operation,
};
use super::materialization::{
    NodeSceneActivationRequest, NodeScenePlanningFault, NodeSceneSessionActivationBootstrap,
    activate_node_scene_with_faults_for_test, node_scene_activation_roles,
};
use super::{NodeSceneBuildFaults, NodeSceneDraftBuild};

/// Mutates actual admitted values after trusted construction and before central validation.
#[derive(Debug, Clone, Copy)]
pub(super) enum SpatialBuildFault {
    CameraCount,
    LightCount,
    InvalidUnitScale,
    CoincidentAxes,
    IncompleteTangentConvention,
    UnexpectedSchemaNameBytes,
    NonFiniteLightTint,
    NonCanonicalZeroEmission,
    CameraAttachmentOutOfRange,
    LightAttachmentOutOfRange,
    OrphanCamera,
    OrphanLight,
}

impl SpatialBuildFault {
    pub(super) const ALL: [Self; 12] = [
        Self::CameraCount,
        Self::LightCount,
        Self::InvalidUnitScale,
        Self::CoincidentAxes,
        Self::IncompleteTangentConvention,
        Self::UnexpectedSchemaNameBytes,
        Self::NonFiniteLightTint,
        Self::NonCanonicalZeroEmission,
        Self::CameraAttachmentOutOfRange,
        Self::LightAttachmentOutOfRange,
        Self::OrphanCamera,
        Self::OrphanLight,
    ];

    pub(super) fn corrupt_spatial(
        self,
        storage: &mut crate::canonical_scene::store::typed_storage::spatial::SpatialStorage,
    ) {
        match self {
            Self::CameraCount => storage.cameras.clear(),
            Self::LightCount => storage.lights.clear(),
            Self::InvalidUnitScale => storage.space.unit_scale_to_meters = Some(0.0),
            Self::CoincidentAxes => storage.space.front_axis = storage.space.up_axis,
            Self::IncompleteTangentConvention => {
                storage.space.tangent_basis = crate::TangentBasisConvention::MikkTspace;
            }
            Self::UnexpectedSchemaNameBytes => {
                storage.space.tangent_basis = crate::TangentBasisConvention::Namespaced {
                    schema_id: crate::ExtensionSchemaId::new("test:spatial_fault", 1).unwrap(),
                };
                storage.space.tangent_handedness_encoding =
                    crate::TangentHandednessEncoding::SignInW;
            }
            Self::NonFiniteLightTint => storage.lights[0].tint.x = f32::NAN,
            Self::NonCanonicalZeroEmission => storage.lights[0].intensity = -0.0,
            _ => {}
        }
    }

    pub(super) fn corrupt_nodes(
        self,
        storage: &mut crate::canonical_scene::store::typed_storage::NodeStorage,
    ) {
        match self {
            Self::CameraAttachmentOutOfRange => storage.nodes[1].camera = Some(1),
            Self::LightAttachmentOutOfRange => storage.nodes[1].light = Some(1),
            Self::OrphanCamera => storage.nodes[1].camera = None,
            Self::OrphanLight => storage.nodes[1].light = None,
            _ => {}
        }
    }
}

#[must_use = "the admitted Scene input must be validated or deliberately dropped"]
pub(in crate::canonical_scene::store) struct NodeSceneTestTransaction<'input> {
    owner: RetainedStoreSessionOutcomeOwner<
        super::super::SceneConstructionOwner,
        SceneConstructionPhase,
        SceneConstructionUnit,
    >,
    request: NodeSceneActivationRequest<'input>,
}

impl<'input> NodeSceneTestTransaction<'input> {
    pub(in crate::canonical_scene::store) fn with_account(
        account: &PhysicalStorageAccount,
        limits: SceneConstructionLimits,
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
    ) -> Result<Self, SceneConstructionStartError> {
        Self::with_account_and_control(
            account,
            limits,
            OperationControl::default(),
            input,
            validation_limits,
        )
    }

    fn with_account_and_control(
        account: &PhysicalStorageAccount,
        limits: SceneConstructionLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
    ) -> Result<Self, SceneConstructionStartError> {
        let envelope = SceneInputEnvelope::try_new(input, validation_limits)?;
        let request = NodeSceneActivationRequest::try_new(input, validation_limits)?;
        let bootstrap = NodeSceneSessionActivationBootstrap::try_new()?;
        let start = try_plan_scene_operation(
            limits,
            control,
            envelope.terminal_attribution_slots(),
            SceneFailureEvidenceWork::exact_single_record(NODE_SCENE_EVIDENCE_FIELD_COUNT),
        )?;
        let raw = SceneStoreOperation::try_start(
            start.control,
            start.ledger_plan,
            account,
            start.start_plan,
        )?;
        let roles = node_scene_activation_roles(start.keys);
        let (outcome_evidence_writer_limits, failure, _evidence) = bootstrap.into_parts();
        let owner = raw.into_session_outcome_owner(
            roles.session_policy(),
            outcome_evidence_writer_limits,
            failure,
        )?;
        Ok(Self { owner, request })
    }

    pub(in crate::canonical_scene::store) fn with_account_and_cancellation(
        account: &PhysicalStorageAccount,
        limits: SceneConstructionLimits,
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
        cancellation: impl baozi_operation::CancellationSignal,
    ) -> Result<Self, SceneConstructionStartError> {
        Self::with_account_and_control(
            account,
            limits,
            OperationControl::default().with_cancellation(cancellation),
            input,
            validation_limits,
        )
    }

    /// Runs the complete fresh Scene authoring transaction through the sole scoped activation rail.
    pub(in crate::canonical_scene::store) fn finish_draft(
        self,
    ) -> Result<NodeSceneDraftBuild, super::super::SceneConstructionError> {
        let Self { owner, request } = self;
        request.activate(
            owner,
            CanonicalValidationProgress::new(
                SceneConstructionPhase::Validation,
                SceneConstructionUnit::WorkUnits,
            ),
            final_progress(),
        )
    }

    pub(super) fn finish_draft_with_faults(
        self,
        faults: NodeSceneBuildFaults,
    ) -> Result<NodeSceneDraftBuild, super::super::SceneConstructionError> {
        let Self { owner, request } = self;
        activate_node_scene_with_faults_for_test(
            request,
            owner,
            CanonicalValidationProgress::new(
                SceneConstructionPhase::Validation,
                SceneConstructionUnit::WorkUnits,
            ),
            final_progress(),
            faults,
        )
    }

    pub(super) fn finish_draft_with_planning_fault(
        self,
        planning: NodeScenePlanningFault,
    ) -> Result<NodeSceneDraftBuild, super::super::SceneConstructionError> {
        self.finish_draft_with_faults(NodeSceneBuildFaults {
            planning: Some(planning),
            ..NodeSceneBuildFaults::none()
        })
    }
}
