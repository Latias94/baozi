use baozi_operation::__retained_store_integration::activation::CanonicalValidationProgress;
use baozi_operation::{OperationControl, PhysicalStorageAccount};

use crate::SceneValidationLimits;

use super::super::super::production::{
    SceneAuthoringBatch, scene_input_terminal_attribution_slots,
};
use super::super::super::schema::NODE_SCENE_EVIDENCE_FIELD_COUNT;
use super::super::{
    SceneConstructionLimits, SceneConstructionPhase, SceneConstructionStartError,
    SceneConstructionUnit, SceneFailureEvidenceWork, SceneOperationStart, SceneStoreOperation,
    try_plan_scene_operation,
};
use super::materialization::{
    NodeSceneActivationRequest, NodeScenePlanningFault, NodeSceneSessionActivationBootstrap,
    NodeSceneTestSlots, activate_node_scene_with_faults_for_test, node_scene_activation_roles,
};
use super::{NodeSceneBuildFaults, NodeSceneDraftBuild};

/// Corrupts retained appearance rows or bytes after authoring admission.
#[derive(Debug, Clone, Copy)]
pub(super) enum AppearanceBuildFault {
    ImageCount,
    EmptyImage,
    ImagePartition,
    InvalidMediaType,
    TextureImage,
    TextureSampler,
    BindingTexture,
    BindingPartition,
    BindingColor,
    BindingChannels,
    BindingStrength,
    DuplicateRole,
    MaterialWorkflow,
    SourceKeyOverlap,
}

impl AppearanceBuildFault {
    pub(super) const ALL: [Self; 14] = [
        Self::ImageCount,
        Self::EmptyImage,
        Self::ImagePartition,
        Self::InvalidMediaType,
        Self::TextureImage,
        Self::TextureSampler,
        Self::BindingTexture,
        Self::BindingPartition,
        Self::BindingColor,
        Self::BindingChannels,
        Self::BindingStrength,
        Self::DuplicateRole,
        Self::MaterialWorkflow,
        Self::SourceKeyOverlap,
    ];

    pub(super) fn corrupt(
        self,
        storage: &mut crate::canonical_scene::store::typed_storage::appearance::AppearanceRows,
    ) {
        use crate::appearance::{
            ColorInterpretation, ShadingModel, TextureChannelSwizzle, TextureStrength,
        };
        use crate::canonical_scene::store::typed_storage::TableRange;
        use crate::canonical_scene::store::typed_storage::appearance::{
            StoredImageIndex, StoredSamplerIndex, StoredTextureIndex,
        };

        match self {
            Self::ImageCount => storage.images.clear(),
            Self::EmptyImage => storage.images[0].payload = TableRange::new(0, 0),
            Self::ImagePartition => {
                let range = storage.images[0].payload;
                storage.images[0].payload = TableRange::new(1, range.len());
            }
            Self::InvalidMediaType => {}
            Self::TextureImage => {
                storage.textures[0].image = StoredImageIndex(storage.images.len() as u32);
            }
            Self::TextureSampler => {
                storage.textures[0].sampler = StoredSamplerIndex(storage.samplers.len() as u32);
            }
            Self::BindingTexture => {
                storage.bindings[0].texture = StoredTextureIndex(storage.textures.len() as u32);
            }
            Self::BindingPartition => storage.materials[0].bindings = TableRange::new(1, 2),
            Self::BindingColor => storage.bindings[0].color = ColorInterpretation::Data,
            Self::BindingChannels => storage.bindings[0].channels = TextureChannelSwizzle::RED,
            Self::BindingStrength => {
                storage.bindings[0].strength = TextureStrength::try_new(2.0).unwrap();
            }
            Self::DuplicateRole => storage.bindings[1] = storage.bindings[0],
            Self::MaterialWorkflow => {
                storage.materials[0].shading = ShadingModel::PbrSpecularGlossiness
            }
            Self::SourceKeyOverlap => storage.bindings[0].source_key = storage.images[0].media_type,
        }
    }
    pub(super) fn corrupt_payloads(
        self,
        payloads: &mut crate::canonical_scene::store::typed_storage::appearance::AppearancePayloads,
    ) {
        if matches!(self, Self::InvalidMediaType) {
            payloads.text[0] = 0xff;
        }
    }
}

/// Corrupts retained animation values after authoring admission has succeeded.
#[derive(Debug, Clone, Copy)]
pub(super) enum AnimationBuildFault {
    ClipCount,
    ChannelPartition,
    SyntheticNodeTarget,
    DuplicateTarget,
    UnorderedTimes,
    NonUnitRotation,
    MorphWidth,
    MissingCubicTangent,
    NonFiniteTangent,
    NegativeDuration,
}

impl AnimationBuildFault {
    pub(super) const ALL: [Self; 10] = [
        Self::ClipCount,
        Self::ChannelPartition,
        Self::SyntheticNodeTarget,
        Self::DuplicateTarget,
        Self::UnorderedTimes,
        Self::NonUnitRotation,
        Self::MorphWidth,
        Self::MissingCubicTangent,
        Self::NonFiniteTangent,
        Self::NegativeDuration,
    ];

    pub(super) fn corrupt(
        self,
        storage: &mut crate::canonical_scene::store::typed_storage::animation::AnimationStorage,
    ) {
        use crate::canonical_scene::store::typed_storage::animation::StoredAnimationTarget;
        use crate::canonical_scene::store::typed_storage::{StoredNodeIndex, TableRange};

        match self {
            Self::ClipCount => storage.clips.clear(),
            Self::ChannelPartition => storage.clips[0].channels = TableRange::new(0, 2),
            Self::SyntheticNodeTarget => {
                storage.channels[0].target =
                    StoredAnimationTarget::NodeTranslation(StoredNodeIndex(0));
            }
            Self::DuplicateTarget => storage.channels[1].target = storage.channels[0].target,
            Self::UnorderedTimes => storage.times[1] = storage.times[0],
            Self::NonUnitRotation => {
                let start = storage.channels[1].values.start();
                storage.values[start + 3] = 0.0;
            }
            Self::MorphWidth => {
                let range = storage.channels[2].values;
                storage.channels[2].values = TableRange::new(range.start(), range.len() - 1);
            }
            Self::MissingCubicTangent => storage.channels[2].in_tangents = None,
            Self::NonFiniteTangent => storage.in_tangents[0] = f32::NAN,
            Self::NegativeDuration => storage.clips[0].duration = -1.0,
        }
    }
}

/// Corrupts real deformation rows and payloads before the central validator observes them.
#[derive(Debug, Clone, Copy)]
pub(super) enum DeformationBuildFault {
    SkinCount,
    SyntheticJoint,
    ProjectiveInverseBind,
    DefinitionRange,
    InfluenceSourceOrder,
    NonFiniteWeight,
    SparseIndex,
    InstanceSkin,
}

impl DeformationBuildFault {
    pub(super) const ALL: [Self; 8] = [
        Self::SkinCount,
        Self::SyntheticJoint,
        Self::ProjectiveInverseBind,
        Self::DefinitionRange,
        Self::InfluenceSourceOrder,
        Self::NonFiniteWeight,
        Self::SparseIndex,
        Self::InstanceSkin,
    ];

    pub(super) fn corrupt(
        self,
        storage: &mut crate::canonical_scene::store::typed_storage::deformation::DeformationStorage,
    ) {
        use crate::canonical_scene::store::typed_storage::TableRange;
        use crate::canonical_scene::store::typed_storage::deformation::StoredSkinIndex;

        match self {
            Self::SkinCount => storage.skins.clear(),
            Self::SyntheticJoint => storage.joints[0].0 = 0,
            Self::ProjectiveInverseBind => storage.inverse_bind_matrices[0].cols[0][3] = 0.5,
            Self::DefinitionRange => {
                let logical = &mut storage.logical_morphs[0];
                logical.definitions = TableRange::new(1, logical.definitions.len());
            }
            Self::InfluenceSourceOrder => storage.influence_source_orders[0] = u32::MAX,
            Self::NonFiniteWeight => storage.influence_weights[0] = f32::NAN,
            Self::SparseIndex => storage.morph_sparse_indices[0] = u32::MAX,
            Self::InstanceSkin => {
                storage.mesh_instances[0].skin = Some(StoredSkinIndex(storage.skins.len() as u32));
            }
        }
    }
}

/// Mutates actual admitted values after trusted construction and before central validation.
#[derive(Debug, Clone, Copy)]
pub(super) enum SpatialBuildFault {
    CameraCount,
    LightCount,
    InvalidUnitScale,
    CoincidentAxes,
    NonFiniteLightTint,
    NonCanonicalZeroEmission,
    CameraAttachmentOutOfRange,
    LightAttachmentOutOfRange,
    OrphanCamera,
    OrphanLight,
}

impl SpatialBuildFault {
    pub(super) const ALL: [Self; 10] = [
        Self::CameraCount,
        Self::LightCount,
        Self::InvalidUnitScale,
        Self::CoincidentAxes,
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
            Self::NonFiniteLightTint => {
                let light = storage.lights[0];
                let mut tint = light.tint();
                tint.x = f32::NAN;
                storage.lights[0] = light.with_emission_parts_for_test(tint, light.intensity());
            }
            Self::NonCanonicalZeroEmission => {
                let light = storage.lights[0];
                storage.lights[0] = light.with_emission_parts_for_test(light.tint(), -0.0);
            }
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
pub(in crate::canonical_scene::store) struct NodeSceneTestTransaction<'input, 'inv, 'account> {
    host: &'inv mut super::super::SceneCanonicalInvocationHost,
    account: &'account PhysicalStorageAccount,
    start: SceneOperationStart,
    bootstrap: NodeSceneSessionActivationBootstrap,
    request: NodeSceneActivationRequest<'input>,
}

impl<'input, 'inv, 'account> NodeSceneTestTransaction<'input, 'inv, 'account> {
    pub(in crate::canonical_scene::store) fn with_account(
        host: &'inv mut super::super::SceneCanonicalInvocationHost,
        account: &'account PhysicalStorageAccount,
        limits: SceneConstructionLimits,
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
    ) -> Result<Self, SceneConstructionStartError> {
        Self::with_account_and_control(
            host,
            account,
            limits,
            OperationControl::default(),
            input,
            validation_limits,
        )
    }

    fn with_account_and_control(
        host: &'inv mut super::super::SceneCanonicalInvocationHost,
        account: &'account PhysicalStorageAccount,
        limits: SceneConstructionLimits,
        control: OperationControl<SceneConstructionPhase, SceneConstructionUnit>,
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
    ) -> Result<Self, SceneConstructionStartError> {
        let terminal_attribution_slots =
            scene_input_terminal_attribution_slots(input, validation_limits)?;
        let request = NodeSceneActivationRequest::try_new(input, validation_limits)?;
        let bootstrap = NodeSceneSessionActivationBootstrap::try_new()?;
        let start = try_plan_scene_operation(
            limits,
            control,
            terminal_attribution_slots,
            SceneFailureEvidenceWork::exact_single_record(NODE_SCENE_EVIDENCE_FIELD_COUNT),
        )?;
        Ok(Self {
            host,
            account,
            start,
            bootstrap,
            request,
        })
    }

    pub(in crate::canonical_scene::store) fn with_account_and_cancellation(
        host: &'inv mut super::super::SceneCanonicalInvocationHost,
        account: &'account PhysicalStorageAccount,
        limits: SceneConstructionLimits,
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
        cancellation: impl baozi_operation::CancellationSignal,
    ) -> Result<Self, SceneConstructionStartError> {
        Self::with_account_and_control(
            host,
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
    ) -> Result<NodeSceneDraftBuild<'inv>, super::super::SceneConstructionError> {
        self.finish_draft_with_faults(NodeSceneBuildFaults::none())
    }

    pub(super) fn finish_draft_with_observer(
        self,
        before_preflight: impl FnOnce(),
    ) -> Result<NodeSceneDraftBuild<'inv>, super::super::SceneConstructionError> {
        self.finish_draft_with_faults_and_observer(NodeSceneBuildFaults::none(), before_preflight)
    }

    pub(super) fn finish_draft_with_faults(
        self,
        faults: NodeSceneBuildFaults,
    ) -> Result<NodeSceneDraftBuild<'inv>, super::super::SceneConstructionError> {
        self.finish_draft_with_faults_and_observer(faults, || {})
    }

    fn finish_draft_with_faults_and_observer(
        self,
        faults: NodeSceneBuildFaults,
        before_preflight: impl FnOnce(),
    ) -> Result<NodeSceneDraftBuild<'inv>, super::super::SceneConstructionError> {
        let Self {
            host,
            account,
            start,
            bootstrap,
            request,
        } = self;
        let mut inputs = Some((start, bootstrap, request));
        let mut before_preflight = Some(before_preflight);
        let mut evidence = None;
        let mut body_error = None;
        let finish = host.run_body(|parts, disposition| {
            let (start, bootstrap, request) = inputs.take().expect("one Scene body");
            let result = (|| {
                let operation = SceneStoreOperation::try_start(
                    parts.startup,
                    start.control,
                    start.ledger_plan,
                    account,
                    start.start_plan,
                )
                .map_err(SceneConstructionStartError::from)?;
                let (writer, failure, _) = bootstrap.into_parts();
                let owner = operation
                    .into_session_outcome_owner(
                        node_scene_activation_roles(start.keys).session_policy(),
                        writer,
                        failure,
                    )
                    .map_err(SceneConstructionStartError::from)?;
                before_preflight.take().unwrap()();
                activate_node_scene_with_faults_for_test(
                    request,
                    owner,
                    NodeSceneTestSlots {
                        output_slot: parts.output,
                        proof_slot: parts.proof,
                        activation_slots: parts.activation,
                        validation_slots: parts.validation,
                    },
                    CanonicalValidationProgress::new(
                        SceneConstructionPhase::Validation,
                        SceneConstructionUnit::WorkUnits,
                    ),
                    disposition,
                    faults,
                )
            })();
            match result {
                Ok(recipe) => evidence = Some(recipe),
                Err(cause) => body_error = Some(cause),
            }
        });
        let finish = match finish {
            Ok(finish) => finish,
            Err(cause) => return Err(body_error.unwrap_or_else(|| cause.into())),
        };
        NodeSceneDraftBuild::from_completion(
            finish.resume(),
            evidence.expect("a recorded Scene disposition retains its evidence recipe"),
        )
    }

    pub(super) fn finish_draft_with_planning_fault(
        self,
        planning: NodeScenePlanningFault,
    ) -> Result<NodeSceneDraftBuild<'inv>, super::super::SceneConstructionError> {
        self.finish_draft_with_faults(NodeSceneBuildFaults {
            planning: Some(planning),
            ..NodeSceneBuildFaults::none()
        })
    }
}
