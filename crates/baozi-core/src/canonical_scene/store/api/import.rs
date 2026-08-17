//! Hidden cross-crate bridge for the U4 canonical import cutover.

use std::fmt;
use std::num::NonZeroU64;

use baozi_operation::__canonical_activation::{
    CanonicalFailureRequirements, CanonicalSuccessRequirements, CanonicalTableShape,
    CanonicalValidationProgress,
};
use baozi_operation::{
    ControlCause, FinalProgress, OperationOwner, ProgressPhase, ProgressUnit,
    RetainedStoreFailureTerminalWorkRequirement, RetainedStoreOperation, StoreDimensionKey,
    StoreLedgerEvidence, StorePhaseKey,
};

pub use crate::identity::SceneGenerationId;

use super::super::construction::node_scene::snapshot::{
    NodeSceneSnapshot, NodeSceneSnapshotError, NodeSceneSnapshotLimits,
};
use super::super::construction::node_scene::{
    DeferredNodeSceneDraftBuild, NodeSceneActivationRequest, NodeSceneActivationRoles,
    NodeSceneConstructionTerminal, NodeSceneDraft, NodeSceneRichFailure, NodeSceneSuccessParts,
    NodeSceneSuccessReport, NodeSceneWriterRoles, PendingNodeSceneDraft, PendingNodeSceneFailure,
    PreparedNodeSceneConstruction, prepare_unactivated_node_scene,
};
use super::super::construction::{
    FAILURE_TERMINAL_EVIDENCE_MEMBERS, SceneConstructionError, SceneConstructionStartError,
    TracerSceneFallback, failure_terminal_work_requirement,
};
use super::super::production::SceneInputEnvelope;
use super::super::schema::NODE_SCENE_EVIDENCE_FIELD_COUNT;

pub use super::scene::ValidatedScene;

pub use super::super::production::input::animation::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationExtrapolationAuthoring, AnimationInterpolationAuthoring, AnimationTargetAuthoring,
    AnimationTimeDomainAuthoring,
};
pub use super::super::production::input::appearance::{
    AppearanceAuthoringBatch, ImageAuthoring, ImageLocalId, MaterialAuthoring,
    MaterialBindingAuthoring, MaterialLocalId, SamplerAuthoring, SamplerLocalId, TextureAuthoring,
    TextureLocalId,
};
pub use super::super::production::input::deformation::{
    DeformationAuthoringBatch, InfluenceAuthoring, InfluenceRowsAuthoring, LogicalMorphAuthoring,
    MeshInstanceDeformationAuthoring, MorphDeltaAuthoring, MorphDeltaPayloadAuthoring,
    MorphTargetDefinitionAuthoring, MorphTargetIdentity, PrimitiveDeformationAuthoring,
    PrimitiveMorphTargetAuthoring, SkinAuthoring, SkinLocalId,
};
pub use super::super::production::input::geometry::{
    AuthoringRange, GeometryAttributeAuthoring, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring,
    GeometryTopologyAuthoring, LogicalMeshLocalId,
};
pub use super::super::production::input::lineage::{
    LineageAuthoringBatch, ObservedResourceAuthoring, OwnerProvenanceAuthoring, ProfileOwner,
    ProvenanceLocalId, ProvenanceNodeAuthoring, ProvenanceOperationAuthoring,
    ResourceIdentityLocalId, SessionOpaqueReason, SourceIdentityAuthoring, SourceIdentityLocalId,
};
pub use super::super::production::input::metadata::{
    MetadataAuthoringBatch, MetadataAuthoringEntry,
};
pub use super::super::production::input::spatial::{
    CameraAuthoring, CameraLocalId, LightAuthoring, LightAuthoringError, LightLocalId,
    SpatialAuthoringBatch,
};
pub use super::super::production::input::{
    NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent, Presence,
    ProfileAuthoringBatch, ResourceSnapshotAuthoring, SceneAuthoringBatch,
};
pub use crate::FormatId;
pub use crate::appearance::{
    AlphaMode, AppearanceBuildError, ColorInterpretation, LegacyMaterialFactors, MaterialFactors,
    PbrMetallicRoughnessFactors, PbrSpecularGlossinessFactors, ShadingModel,
    SupplementalPbrFactors, TextureBlendOperation, TextureChannel, TextureChannelMapping,
    TextureChannelSwizzle, TextureCoordinateSource, TextureRole, TextureSourceKey, TextureStrength,
    TextureTransform,
};
pub use crate::attribute::{AttributeDomain, AttributeSemantic, CustomAttributeSemantic};
pub use crate::deform::{MorphDeltaSemantic, SkinMatrixSpaces};
pub use crate::entity::{
    AuthoredTransform, CameraProjection, EntityName, LightKind, MAX_ENTITY_NAME_BYTES,
};
pub use crate::extension::{ExtensionMediaType, ExtensionValueRef};
pub use crate::image::{MipmapFilter, SamplerState, TexelFilter, TextureAddressMode};
pub use crate::math::{Color, Mat4, Quat, Vec2, Vec3};
pub use crate::namespaced::NamespacedName;
pub use crate::provenance::{
    ContentDigest, DigestAlgorithm, EvidenceDigest, OperationEvidenceSummary, OperationKind,
    OptionsDigest, ResourceIdentity, SourceDisplayName, SourceObjectKind, SourceObjectLocator,
};
pub use crate::scene_space::{
    Axis, FrontFaceWinding, Handedness, SceneSpace, TangentBasisConvention,
    TangentHandednessEncoding, TransformConvention, UvOrigin,
};
pub use crate::validation_control::SceneValidationLimits;

/// Roleless complete-batch request which can be consumed only with one retained operation.
#[must_use = "the Scene activation request must be consumed by one import session owner"]
pub struct SceneActivationRequest<'input>(NodeSceneActivationRequest<'input>);

/// Returns the maximum canonical block count admitted under `limits`.
pub fn maximum_scene_canonical_blocks(limits: SceneValidationLimits) -> Option<usize> {
    SceneInputEnvelope::maximum_canonical_blocks(limits)
}

/// Returns the protected structural-work ceiling for any fresh Scene admitted under `limits`.
pub fn maximum_scene_structural_work(limits: SceneValidationLimits) -> Option<u64> {
    let canonical_blocks = maximum_scene_canonical_blocks(limits)?;
    let canonical = CanonicalTableShape::new(canonical_blocks, canonical_blocks).ok()?;
    let success =
        CanonicalSuccessRequirements::new(canonical, canonical_blocks, NonZeroU64::MIN).ok()?;
    let failure_blocks = FAILURE_TERMINAL_EVIDENCE_MEMBERS;
    let failure = CanonicalFailureRequirements::new(
        CanonicalTableShape::new(failure_blocks, failure_blocks).ok()?,
    );
    success.structural_work_requirement(&failure).ok()
}

impl<'input> SceneActivationRequest<'input> {
    /// Validates the bounded input envelope without acquiring operation or storage authority.
    pub fn try_new(
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
    ) -> Result<Self, SceneActivationRequestError> {
        NodeSceneActivationRequest::try_new(input, validation_limits)
            .map(Self)
            .map_err(SceneActivationRequestError)
    }

    /// Consumes the whole request and binds it to the host's sole operation and role mapping.
    pub fn activate<O, PP, U>(
        self,
        operation: RetainedStoreOperation<O, PP, U>,
        roles: SceneActivationRoles,
        validation_progress: CanonicalValidationProgress<PP, U>,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<SceneDraftBuild<O, PP, U>, SceneActivationError>
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        self.0
            .activate_deferred(operation, roles.0, validation_progress, final_progress)
            .map(SceneDraftBuild::from_node_scene)
            .map_err(SceneActivationError)
    }
}

/// One writer's host-owned phase, capacity, and attribution roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneWriterRoles(NodeSceneWriterRoles);

impl SceneWriterRoles {
    /// Binds one roleless writer envelope to host-owned store keys.
    pub const fn new(
        phase: StorePhaseKey,
        capacity_dimension: StoreDimensionKey,
        attribution_dimension: StoreDimensionKey,
    ) -> Self {
        Self(NodeSceneWriterRoles::new(
            phase,
            capacity_dimension,
            attribution_dimension,
        ))
    }
}

/// Complete host-owned role mapping for one canonical Scene activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneActivationRoles(NodeSceneActivationRoles);

impl SceneActivationRoles {
    /// Creates a complete key mapping without accepting request-derived limits or execution data.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        canonical_writer: SceneWriterRoles,
        outcome_evidence_writer: SceneWriterRoles,
        materialization_work_dimension: StoreDimensionKey,
        materialization_scratch_dimension: StoreDimensionKey,
        validation_phase: StorePhaseKey,
        validation_work_dimension: StoreDimensionKey,
        validation_scratch_dimension: StoreDimensionKey,
        canonical_destination_dimension: StoreDimensionKey,
        outcome_evidence_destination_dimension: StoreDimensionKey,
    ) -> Self {
        Self(NodeSceneActivationRoles::new(
            canonical_writer.0,
            outcome_evidence_writer.0,
            materialization_work_dimension,
            materialization_scratch_dimension,
            validation_phase,
            validation_work_dimension,
            validation_scratch_dimension,
            canonical_destination_dimension,
            outcome_evidence_destination_dimension,
        ))
    }

    /// Projects exact retained Scene string bytes into the import host's canonical string role.
    pub const fn with_canonical_string_dimension(mut self, dimension: StoreDimensionKey) -> Self {
        self.0 = self.0.with_canonical_string_dimension(dimension);
        self
    }
}

/// Non-owning descriptor for one materialized canonical Scene directory.
///
/// The descriptor is deliberately neither `Clone` nor `Copy`. Its matching retained owner stays
/// in the import session until the parser returns this exact value.
#[must_use = "the Scene draft must be returned to its import session"]
pub struct SceneDraft(NodeSceneDraft);

/// Structural owner for the retained blocks referenced by one [`SceneDraft`].
///
/// Preparation consumes both values. A missing or foreign descriptor becomes an ordinary frozen
/// failure before generation reservation.
#[must_use = "the pending Scene draft must be prepared or deliberately dropped"]
pub struct PendingSceneDraftOwner<O, PP, U>(PendingNodeSceneDraft<O, PP, U>)
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit;

impl<O, PP, U> PendingSceneDraftOwner<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Verifies the parser result, reserves one generation only for an exact match, and freezes
    /// the terminal publication.
    pub fn prepare(
        self,
        draft: Option<SceneDraft>,
    ) -> Result<PreparedSceneConstruction, SceneActivationError> {
        let (prepared, control_cause) = match draft {
            Some(draft) => self
                .0
                .prepare_import_with_generation_source(draft.0, SceneGenerationId::reserve),
            None => self.0.prepare_import_missing_draft(),
        }
        .map_err(SceneActivationError)?;
        Ok(PreparedSceneConstruction(prepared, control_cause))
    }
}

/// Linear failure continuation retained until parser-owned storage has been released.
#[must_use = "the pending Scene failure must be prepared or deliberately dropped"]
pub struct PendingSceneFailureOwner<O, PP, U>(PendingNodeSceneFailure<O, PP, U>)
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit;

impl<O, PP, U> PendingSceneFailureOwner<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Builds, validates, and freezes the terminal failure after parser storage is quiescent.
    pub fn prepare(self) -> Result<PreparedSceneConstruction, SceneActivationError> {
        let (prepared, authoring_rejection, control_cause) =
            self.0.prepare().map_err(SceneActivationError)?;
        drop(authoring_rejection);
        Ok(PreparedSceneConstruction(prepared, control_cause))
    }
}

/// Opaque activation result which separates the parser descriptor from its retained owner.
#[must_use = "the Scene draft build must be installed in its import session"]
#[allow(
    clippy::large_enum_variant,
    reason = "the failure owner stays inline so fail-closed transitions do not allocate"
)]
pub enum SceneDraftBuild<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Materialization succeeded and awaits the parser's exact descriptor.
    Draft {
        draft: SceneDraft,
        owner: PendingSceneDraftOwner<O, PP, U>,
    },
    /// Activation retained an authoritative failure continuation for later preparation.
    Failure {
        authoring_rejected: bool,
        draft_built: bool,
        control_cause: Option<ControlCause>,
        // Keep the physical continuation last so future classification fields cannot unwind after
        // its containment owner has already been released.
        owner: PendingSceneFailureOwner<O, PP, U>,
    },
}

impl<O, PP, U> SceneDraftBuild<O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn from_node_scene(build: DeferredNodeSceneDraftBuild<O, PP, U>) -> Self {
        match build {
            DeferredNodeSceneDraftBuild::Draft { draft, owner } => Self::Draft {
                draft: SceneDraft(draft),
                owner: PendingSceneDraftOwner(owner),
            },
            DeferredNodeSceneDraftBuild::Failure {
                authoring_rejected,
                draft_built,
                control_cause,
                owner,
            } => Self::Failure {
                authoring_rejected,
                draft_built,
                control_cause,
                owner: PendingSceneFailureOwner(owner),
            },
        }
    }
}

/// Frozen publication whose remaining handoff is a pure ownership move.
#[must_use = "the prepared Scene construction must be handed off or deliberately dropped"]
pub struct PreparedSceneConstruction(PreparedNodeSceneConstruction, Option<ControlCause>);

impl PreparedSceneConstruction {
    /// Moves the frozen publication into its final canonical Scene or failure owner.
    pub fn handoff(self) -> SceneConstructionTerminal {
        SceneConstructionTerminal(self.0.handoff(), self.1)
    }
}

/// Freezes the pre-admitted fallback when an import ends before submitting a Scene request.
///
/// This consumes the same retained operation that accounted producer work. It creates no empty
/// Scene, generation, canonical commitment, or second ledger.
pub fn prepare_unactivated_scene<O, PP, U>(
    operation: RetainedStoreOperation<O, PP, U>,
    stable_code: u32,
) -> Result<PreparedSceneConstruction, SceneActivationError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    prepare_unactivated_node_scene(operation, stable_code)
        .map(|prepared| PreparedSceneConstruction(prepared, None))
        .map_err(SceneActivationError)
}

/// Final canonical Scene construction outcome retained by the import session.
#[must_use = "the terminal owns the exact canonical Scene construction outcome"]
pub struct SceneConstructionTerminal(NodeSceneConstructionTerminal, Option<ControlCause>);

impl SceneConstructionTerminal {
    /// Returns whether canonical Scene construction and joint publication succeeded.
    pub const fn is_success(&self) -> bool {
        matches!(self.0, NodeSceneConstructionTerminal::Success(_))
    }

    /// Returns the retained canonical member count for a successful Scene.
    pub fn canonical_block_count(&self) -> Option<usize> {
        match &self.0 {
            NodeSceneConstructionTerminal::Success(NodeSceneSuccessParts { scene, .. }) => {
                Some(scene.canonical_block_count())
            }
            NodeSceneConstructionTerminal::RichFailure(_)
            | NodeSceneConstructionTerminal::MinimalFallback(_) => None,
        }
    }

    /// Borrows the unique frozen ledger evidence for this terminal outcome.
    pub const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        match &self.0 {
            NodeSceneConstructionTerminal::Success(parts) => parts.report.ledger_evidence(),
            NodeSceneConstructionTerminal::RichFailure(failure) => failure.ledger_evidence(),
            NodeSceneConstructionTerminal::MinimalFallback(fallback) => fallback.ledger_evidence(),
        }
    }

    /// Returns the exact operation-control cause retained alongside a Control terminal.
    pub const fn control_cause(&self) -> Option<&ControlCause> {
        self.1.as_ref()
    }

    /// Consumes the terminal into the exact retained owners published by canonical construction.
    ///
    /// This is an ownership projection only: it does not copy ledger evidence, inspect raw
    /// canonical storage, or perform another publication step.
    pub fn into_parts(self) -> SceneConstructionTerminalParts {
        match self.0 {
            NodeSceneConstructionTerminal::Success(NodeSceneSuccessParts { scene, report }) => {
                SceneConstructionTerminalParts::Success {
                    scene: ValidatedScene::from_node_scene(scene),
                    report: SceneSuccessReport(report),
                }
            }
            NodeSceneConstructionTerminal::RichFailure(failure) => {
                SceneConstructionTerminalParts::RichFailure(SceneRichFailure(failure))
            }
            NodeSceneConstructionTerminal::MinimalFallback(fallback) => {
                SceneConstructionTerminalParts::MinimalFallback(SceneMinimalFallback(fallback))
            }
        }
    }
}

/// Consuming projection of the exact owners retained by one terminal publication.
#[must_use = "the terminal parts own the published canonical Scene or failure evidence"]
#[allow(
    clippy::large_enum_variant,
    reason = "published owners remain inline so the projection is a pure ownership move"
)]
pub enum SceneConstructionTerminalParts {
    /// Independently owned canonical Scene and successful outcome evidence.
    Success {
        /// Centrally validated canonical Scene owner.
        scene: ValidatedScene,
        /// Outcome evidence published in the same joint commit as the canonical Scene.
        report: SceneSuccessReport,
    },
    /// Structured failure evidence published without canonical Scene ownership.
    RichFailure(SceneRichFailure),
    /// Bounded fail-closed owner used when rich evidence could not be published.
    MinimalFallback(SceneMinimalFallback),
}

impl ValidatedScene {
    /// Captures a bounded semantic snapshot without exposing retained storage or validation proof.
    pub fn capture_snapshot(
        &self,
        limits: SceneSnapshotLimits,
    ) -> Result<SceneSnapshot, SceneSnapshotError> {
        self.as_node_scene()
            .capture_snapshot(limits.0, None)
            .map(SceneSnapshot)
            .map_err(SceneSnapshotError)
    }
}

/// Hard bounds for one semantic Scene snapshot capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneSnapshotLimits(NodeSceneSnapshotLimits);

impl SceneSnapshotLimits {
    /// Validates all allocation, collection-preview, and traversal-work limits before capture.
    pub fn try_new(
        max_preview_bytes: usize,
        max_collection_items: usize,
        max_work: u64,
    ) -> Result<Self, SceneSnapshotError> {
        NodeSceneSnapshotLimits::try_new(max_preview_bytes, max_collection_items, max_work)
            .map(Self)
            .map_err(SceneSnapshotError)
    }
}

/// Complete canonical digest, bounded preview, and semantic totals for one retained Scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneSnapshot(NodeSceneSnapshot);

impl SceneSnapshot {
    /// Returns the complete canonical digest.
    pub const fn digest(&self) -> [u8; 32] {
        self.0.digest
    }

    /// Returns the human-reviewable prefix emitted from the same semantic traversal as the digest.
    pub fn preview(&self) -> &str {
        &self.0.preview.text
    }

    /// Returns whether the preview omitted bytes because of its configured byte limit.
    pub const fn preview_bytes_truncated(&self) -> bool {
        self.0.preview.bytes_truncated
    }

    /// Returns whether the preview omitted collection items because of its configured item limit.
    pub const fn preview_items_truncated(&self) -> bool {
        self.0.preview.items_truncated
    }

    /// Returns the total canonical node count.
    pub const fn node_count(&self) -> usize {
        self.0.totals.node_count
    }

    /// Returns the total root-node count.
    pub const fn root_count(&self) -> usize {
        self.0.totals.root_count
    }

    /// Returns the total material count.
    pub const fn material_count(&self) -> usize {
        self.0.totals.material_count
    }

    /// Returns the total image count.
    pub const fn image_count(&self) -> usize {
        self.0.totals.image_count
    }

    /// Returns the total sampler count.
    pub const fn sampler_count(&self) -> usize {
        self.0.totals.sampler_count
    }

    /// Returns the total texture count.
    pub const fn texture_count(&self) -> usize {
        self.0.totals.texture_count
    }

    /// Returns the total material-binding count.
    pub const fn material_binding_count(&self) -> usize {
        self.0.totals.material_binding_count
    }

    /// Returns the total logical-mesh count.
    pub const fn logical_mesh_count(&self) -> usize {
        self.0.totals.logical_mesh_count
    }

    /// Returns the total primitive count.
    pub const fn primitive_count(&self) -> usize {
        self.0.totals.primitive_count
    }

    /// Returns the total geometry-attribute count.
    pub const fn attribute_count(&self) -> usize {
        self.0.totals.attribute_count
    }

    /// Returns the total mesh-instance count.
    pub const fn mesh_instance_count(&self) -> usize {
        self.0.totals.mesh_instance_count
    }

    /// Returns the total number of explicitly retained primitive indices.
    pub const fn primitive_index_count(&self) -> u64 {
        self.0.totals.primitive_index_count
    }

    /// Returns the total number of retained polygon-boundary entries.
    pub const fn polygon_boundary_count(&self) -> u64 {
        self.0.totals.polygon_boundary_count
    }

    /// Returns the total number of canonical geometry scalars.
    pub const fn canonical_scalar_count(&self) -> u64 {
        self.0.totals.canonical_scalar_count
    }

    /// Returns the total canonical f32 scalar count across geometry attributes.
    pub const fn f32_scalar_count(&self) -> u64 {
        self.0.totals.f32_scalar_count
    }

    /// Returns the total canonical u32 scalar count across geometry attributes.
    pub const fn u32_scalar_count(&self) -> u64 {
        self.0.totals.u32_scalar_count
    }

    /// Returns the total canonical i32 scalar count across geometry attributes.
    pub const fn i32_scalar_count(&self) -> u64 {
        self.0.totals.i32_scalar_count
    }

    /// Returns the total encoded image payload bytes.
    pub const fn image_payload_bytes(&self) -> u64 {
        self.0.totals.image_payload_bytes
    }

    /// Returns the total material-factor scalar count.
    pub const fn material_factor_scalar_count(&self) -> u64 {
        self.0.totals.material_factor_scalar_count
    }

    /// Returns the total profile metadata entry count.
    pub const fn profile_metadata_entry_count(&self) -> usize {
        self.0.totals.profile_metadata_entry_count
    }
}

/// Snapshot capture failed before producing a complete digest and totals.
#[derive(Debug)]
pub struct SceneSnapshotError(NodeSceneSnapshotError);

impl fmt::Display for SceneSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for SceneSnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// Opaque owner of outcome evidence jointly published with a successful Scene.
#[must_use = "the success report owns published outcome evidence"]
pub struct SceneSuccessReport(NodeSceneSuccessReport);

impl SceneSuccessReport {
    /// Returns the number of retained outcome-evidence blocks.
    pub fn outcome_evidence_block_count(&self) -> usize {
        self.0.outcome_evidence_block_count()
    }

    /// Borrows the unique frozen ledger evidence for this successful outcome.
    pub const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        self.0.ledger_evidence()
    }
}

/// Opaque owner of one structured canonical-construction failure publication.
#[must_use = "the rich failure owns published outcome evidence"]
pub struct SceneRichFailure(NodeSceneRichFailure);

impl SceneRichFailure {
    /// Returns the number of retained outcome-evidence blocks.
    pub fn outcome_evidence_block_count(&self) -> usize {
        self.0.outcome_evidence_block_count()
    }

    /// Borrows the unique frozen ledger evidence for this failed outcome.
    pub const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        self.0.ledger_evidence()
    }
}

/// Opaque owner of one bounded fail-closed canonical-construction publication.
#[must_use = "the fallback owns its bounded published failure state"]
pub struct SceneMinimalFallback(TracerSceneFallback);

impl SceneMinimalFallback {
    /// Returns the stable failure code retained by the bounded fallback.
    pub fn stable_code(&self) -> u32 {
        self.0.fallback_code()
    }

    /// Borrows the unique frozen ledger evidence for this fail-closed outcome.
    pub const fn ledger_evidence(&self) -> &StoreLedgerEvidence {
        self.0.ledger_evidence()
    }
}

/// Returns the exact protected terminal-work requirement shared by every NodeScene request.
pub fn scene_failure_terminal_work_requirement()
-> Result<RetainedStoreFailureTerminalWorkRequirement, baozi_operation::StorePlanError> {
    failure_terminal_work_requirement()
}

/// Returns the exact maximum number of retained members in rich-failure evidence.
pub const fn scene_failure_evidence_member_count() -> usize {
    FAILURE_TERMINAL_EVIDENCE_MEMBERS
}

/// Returns the exact per-pass field work for rich-failure construction and validation.
pub const fn scene_failure_evidence_field_work() -> u64 {
    NODE_SCENE_EVIDENCE_FIELD_COUNT as u64
}

/// Authority-free request construction failure.
#[derive(Debug)]
pub struct SceneActivationRequestError(SceneConstructionStartError);

impl fmt::Display for SceneActivationRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for SceneActivationRequestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// Fail-closed activation or terminal preparation error.
#[derive(Debug)]
pub struct SceneActivationError(SceneConstructionError);

impl fmt::Display for SceneActivationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for SceneActivationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_projection_requires_ownership_and_exposes_only_safe_views() {
        fn accepts_internal_scene(_: &super::super::ValidatedScene) {}

        let _: fn(&ValidatedScene) = accepts_internal_scene;
        let _: fn(SceneConstructionTerminal) -> SceneConstructionTerminalParts =
            SceneConstructionTerminal::into_parts;
        let _: fn(&ValidatedScene) -> SceneGenerationId = ValidatedScene::generation;
        let _: fn(&ValidatedScene) -> usize = ValidatedScene::canonical_block_count;
        let _: fn(&SceneSuccessReport) -> usize = SceneSuccessReport::outcome_evidence_block_count;
        let _: for<'a> fn(&'a SceneSuccessReport) -> &'a StoreLedgerEvidence =
            SceneSuccessReport::ledger_evidence;
        let _: fn(&SceneRichFailure) -> usize = SceneRichFailure::outcome_evidence_block_count;
        let _: for<'a> fn(&'a SceneRichFailure) -> &'a StoreLedgerEvidence =
            SceneRichFailure::ledger_evidence;
        let _: fn(&SceneMinimalFallback) -> u32 = SceneMinimalFallback::stable_code;
        let _: for<'a> fn(&'a SceneMinimalFallback) -> &'a StoreLedgerEvidence =
            SceneMinimalFallback::ledger_evidence;
    }

    #[test]
    fn hidden_import_surface_constructs_the_complete_authoring_batch() {
        use crate::__canonical_import as bridge;

        let scene_space = bridge::SceneSpace::unknown();
        let input = bridge::SceneAuthoringBatch::new(
            bridge::SpatialAuthoringBatch::new(&scene_space, &[], &[]),
            bridge::NodeAuthoringBatch::new(bridge::Presence::Present(&[]), &[]),
            bridge::AppearanceAuthoringBatch::empty(),
            bridge::GeometryAuthoringBatch::empty(),
            bridge::DeformationAuthoringBatch::empty(),
            bridge::AnimationAuthoringBatch::empty(),
            bridge::ProfileAuthoringBatch::new(
                bridge::LineageAuthoringBatch::new(&[], &[], &[], &[]),
                bridge::MetadataAuthoringBatch::empty(),
            ),
        );
        let _: bridge::SceneAuthoringBatch<'_> = input;

        let _: fn(u32) -> bridge::NodeLocalId = bridge::NodeLocalId::new;
        let _: fn(bridge::NodeLocalId, u32) -> bridge::NodeParent = bridge::NodeParent::new;
        let _: fn(u32, u32) -> bridge::AuthoringRange = bridge::AuthoringRange::new;

        let _ = bridge::AnimationChannelAuthoring::new(
            bridge::AnimationTargetAuthoring::NodeTranslation(bridge::NodeLocalId::new(0)),
            bridge::AnimationInterpolationAuthoring::Linear,
            bridge::AnimationExtrapolationAuthoring::SourceDefault,
            bridge::AnimationExtrapolationAuthoring::SourceDefault,
            &[],
            &[],
            bridge::Presence::Absent,
            bridge::Presence::Absent,
        );
        let _: fn(
            f64,
            bridge::AnimationTimeDomainAuthoring,
            bridge::AuthoringRange,
        ) -> bridge::AnimationClipAuthoring = bridge::AnimationClipAuthoring::new;

        let _: fn(u32) -> bridge::ImageLocalId = bridge::ImageLocalId::new;
        let _: fn(u32) -> bridge::SamplerLocalId = bridge::SamplerLocalId::new;
        let _: fn(u32) -> bridge::TextureLocalId = bridge::TextureLocalId::new;
        let _: fn(u32) -> bridge::MaterialLocalId = bridge::MaterialLocalId::new;
        let _ = bridge::ImageAuthoring::session_resource(
            bridge::ResourceIdentityLocalId::new(0),
            bridge::Presence::Absent,
        );
        let _: fn(bridge::SamplerState) -> bridge::SamplerAuthoring = bridge::SamplerAuthoring::new;
        let _: fn(bridge::ImageLocalId, bridge::SamplerLocalId) -> bridge::TextureAuthoring =
            bridge::TextureAuthoring::new;
        let role = bridge::TextureRole::BaseColor;
        let coordinate_source = bridge::TextureCoordinateSource::UvSet(0);
        let channel_mapping =
            bridge::TextureChannelMapping::swizzle(bridge::TextureChannelSwizzle::RGBA);
        let color_interpretation = bridge::ColorInterpretation::SrgbColor;
        let blend = bridge::TextureBlendOperation::Replace;
        assert!(
            bridge::MaterialBindingAuthoring::try_from_parts(
                bridge::TextureLocalId::new(0),
                &role,
                &coordinate_source,
                &channel_mapping,
                &color_interpretation,
                bridge::TextureTransform::IDENTITY,
                bridge::TextureStrength::ONE,
                &blend,
                None,
            )
            .is_ok()
        );
        assert!(
            bridge::MaterialAuthoring::try_from_parts(
                bridge::AuthoringRange::new(0, 0),
                &bridge::ShadingModel::Unspecified,
                &bridge::AlphaMode::Opaque,
                false,
                bridge::MaterialFactors::empty(),
            )
            .is_ok()
        );

        let _: fn(u32) -> bridge::LogicalMeshLocalId = bridge::LogicalMeshLocalId::new;
        let _ = bridge::GeometryAttributeAuthoring::f32(
            bridge::AttributeSemantic::Position,
            bridge::AttributeDomain::Vertex,
            3,
            &[],
            bridge::Presence::Absent,
        );

        let _: fn(u32) -> bridge::SkinLocalId = bridge::SkinLocalId::new;
        let _: fn(u32) -> bridge::MorphTargetIdentity = bridge::MorphTargetIdentity::new;
        let _: fn(u32, f32, u32) -> bridge::InfluenceAuthoring = bridge::InfluenceAuthoring::new;
        let _ = bridge::MorphDeltaAuthoring::new(
            bridge::MorphDeltaSemantic::Position,
            bridge::MorphDeltaPayloadAuthoring::Dense { values: &[] },
        );

        let _: fn(u32) -> bridge::ResourceIdentityLocalId = bridge::ResourceIdentityLocalId::new;
        let _: fn(u32) -> bridge::SourceIdentityLocalId = bridge::SourceIdentityLocalId::new;
        let _: fn(u32) -> bridge::ProvenanceLocalId = bridge::ProvenanceLocalId::new;
        let resource = bridge::ResourceIdentity::new("tests/authoring-bridge")
            .expect("resource identity must be valid");
        let observed_resource = bridge::ObservedResourceAuthoring::session_opaque(
            &resource,
            bridge::SessionOpaqueReason::CallerLabel,
        );
        let resources = [observed_resource];
        let resource_snapshots = [bridge::ResourceSnapshotAuthoring::snapshotted(&[])];
        let _ = bridge::ProfileAuthoringBatch::new(
            bridge::LineageAuthoringBatch::new(&resources, &[], &[], &[]),
            bridge::MetadataAuthoringBatch::empty(),
        )
        .with_resource_snapshots(&resource_snapshots);
        let format = bridge::FormatId::new("test").expect("format ID must be valid");
        let locator = bridge::SourceObjectLocator::indexed(bridge::SourceObjectKind::Scene, 0);
        let _ = bridge::SourceIdentityAuthoring::new(
            bridge::ResourceIdentityLocalId::new(0),
            &format,
            &locator,
        );

        let key = bridge::NamespacedName::new("baozi-tests:authoring-bridge")
            .expect("metadata key must be valid");
        let _ = bridge::MetadataAuthoringEntry::new(
            bridge::ProfileOwner::Scene,
            &key,
            bridge::ExtensionValueRef::Bool(true),
        );

        let _: fn(
            bridge::LightKind,
            bridge::Color,
            f32,
        ) -> Result<bridge::LightAuthoring, bridge::LightAuthoringError> =
            bridge::LightAuthoring::try_from_scaled_linear_srgb;
    }
}
