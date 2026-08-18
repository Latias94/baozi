use baozi_operation::__canonical_activation::CanonicalValidationProgress;
use baozi_operation::{OperationControl, PhysicalStorageAccount};

use crate::SceneValidationLimits;

use super::super::super::production::SceneAuthoringBatch;
use super::super::super::schema::NODE_SCENE_EVIDENCE_FIELD_COUNT;
use super::super::{
    SceneConstructionLimits, SceneConstructionPhase, SceneConstructionStartError,
    SceneConstructionUnit, SceneFailureEvidenceWork, SceneStoreKeys, SceneStoreOperation,
    final_progress, try_plan_scene_operation,
};
use super::materialization::{
    NodeSceneActivationRequest, NodeScenePlanningFault, activate_node_scene_with_faults_for_test,
    node_scene_activation_roles,
};
use super::{NodeSceneBuildFaults, NodeSceneDraftBuild};

#[must_use = "the admitted Scene input must be validated or deliberately dropped"]
pub(in crate::canonical_scene::store) struct NodeSceneTestTransaction<'input> {
    raw: SceneStoreOperation,
    keys: SceneStoreKeys,
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
        let request = NodeSceneActivationRequest::try_new(input, validation_limits)?;
        let terminal_attribution_slots = request.canonical_writer_limits().attribution_slots();
        let start = try_plan_scene_operation(
            limits,
            control,
            terminal_attribution_slots,
            SceneFailureEvidenceWork::exact_single_record(NODE_SCENE_EVIDENCE_FIELD_COUNT),
        )?;
        let raw = SceneStoreOperation::try_start(
            start.control,
            start.ledger_plan,
            account,
            start.start_plan,
        )?;
        Ok(Self {
            raw,
            keys: start.keys,
            request,
        })
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
        let Self { raw, keys, request } = self;
        request.activate(
            raw,
            node_scene_activation_roles(keys),
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
        let Self { raw, keys, request } = self;
        activate_node_scene_with_faults_for_test(
            request,
            raw,
            node_scene_activation_roles(keys),
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
