//! Concrete Scene-edit bootstrap and source-bound semantic transaction.
//!
//! This module introduces no operation owner: postprocess starts the derived runtime and drives the semantic/materialization
//! lanes. Existing graph, profile, central validation, and publication wiring will consume its plan.

mod evidence;
mod publication;
mod schema;
mod space;
mod validation;
use super::plan::{BoundPendingNodeSceneEdit, NodeSceneEditPlanError, NodeSceneEditProgram};
use crate::canonical_scene::store::construction::node_scene::NodeSceneDraft;
use crate::canonical_scene::store::construction::node_scene::materialization::{
    NodeSceneActivationOutput, NodeSceneEvidenceRecipe, NodeSceneLaneOutput,
};
use crate::canonical_scene::store::schema::SceneDirectorySchema;
use baozi_operation::__retained_store_integration::activation::CanonicalMaterialization;
use baozi_operation::__retained_store_integration::producer::{
    CanonicalActivationDescription, CanonicalActivationPlanSealer, CanonicalSealedActivationPlan,
    CanonicalSuccessRequirements,
};
use baozi_operation::RecordBlock;
pub use publication::*;
pub use space::{
    ResolvedSceneSpaceConversion, SceneSpaceConversionIssue, resolve_scene_space_conversion,
};
pub use validation::SceneEditValidationProof;

pub use schema::{
    SceneEditEvidenceTag, SceneGeometryCornerOriginsTag, SceneGeometryFaceOffsetsTag,
};

use baozi_operation::__retained_store_integration::activation::{
    CanonicalFailureRequirements, CanonicalLane, CanonicalReservation, CanonicalSemanticWorkSession,
};
use baozi_operation::__retained_store_integration::authority::{
    CanonicalInheritanceSource, DerivedRetainedStoreOperation, RetainedStoreOperation,
    RetainedStoreStartError, RetainedStoreStartPlan,
};
use baozi_operation::__retained_store_integration::producer::{
    CanonicalActivationRequest, CanonicalTableShape,
};
use baozi_operation::{
    CapacityPlan, OperationControl, OperationOwner, OutcomeEvidenceRecordRef,
    OutcomeEvidenceRecordRowWriter, ProgressPhase, ProgressUnit, RecordPlan,
    RetainedStoreBatchLimits, RetainedStoreFailureTerminalWorkPlan,
    RetainedStoreFailureTerminalWorkRequirement, StoreBuildError, StoreLedgerPlan, StorePlanError,
    StoreRuntimeError,
};
use thiserror::Error;

use crate::SceneValidationLimits;
use crate::canonical_scene::store::api::scene::ValidatedScene;
use crate::canonical_scene::store::construction::ConstructionBrand;
use crate::canonical_scene::store::construction::node_scene::geometry::map_build;
use crate::canonical_scene::store::construction::node_scene::materialization::NodeSceneSessionActivationBootstrap;
use crate::canonical_scene::store::schema::{
    NODE_SCENE_EVIDENCE_FIELD_COUNT, NodeSceneOutcomeEvidenceSchema,
};
use crate::canonical_scene::store::typed_storage::build::TypedStorageBuildControl;
use crate::canonical_scene::store::typed_storage::edit::normals::{
    GeometryNormalsPlan, NormalWritePolicy, TangentPolicy,
};
use crate::canonical_scene::store::typed_storage::edit::space::{
    SceneSpacePlan, SceneSpacePlanError,
};
use crate::canonical_scene::store::typed_storage::edit::space_animation::AnimationSpacePlanError;
use crate::canonical_scene::store::typed_storage::edit::space_deformation::{
    DeformationSpaceNumericLocation, DeformationSpacePlanError,
};
use crate::canonical_scene::store::typed_storage::edit::space_geometry::GeometrySpacePlanError;
use crate::canonical_scene::store::typed_storage::edit::space_nodes::NodeSpacePlanError;
use crate::canonical_scene::store::typed_storage::edit::space_spatial::SpatialSpacePlanError;
use crate::canonical_scene::store::typed_storage::edit::{
    GeometryTriangulationPlan, TriangulationOrigins,
};
use crate::geometry::TopologyKind;
use crate::identity::{PrimitiveId, SceneGenerationId};
use crate::provenance::OptionsDigest;

use super::SceneEditCompileError;

const SUCCESS_EVIDENCE_MEMBERS: usize = 4;
const FAILURE_EVIDENCE_MEMBERS: usize = 2;
const FAILURE_FIELD_WORK: u64 = (schema::ROOT_FIELDS + NODE_SCENE_EVIDENCE_FIELD_COUNT) as u64;

/// Borrowed origins for one changed polygon primitive. All indices are source primitive-local.
#[derive(Clone, Copy)]
pub struct SceneTriangulationReplacement<'a> {
    target: PrimitiveId,
    triangles: &'a [[u32; 3]],
    face_offsets: &'a [u32],
    options_digest: OptionsDigest,
}

impl<'a> SceneTriangulationReplacement<'a> {
    pub const fn new(
        target: PrimitiveId,
        source_corner_triangles: &'a [[u32; 3]],
        source_face_triangle_offsets: &'a [u32],
        options_digest: OptionsDigest,
    ) -> Self {
        Self {
            target,
            triangles: source_corner_triangles,
            face_offsets: source_face_triangle_offsets,
            options_digest,
        }
    }

    pub const fn target(self) -> PrimitiveId {
        self.target
    }
    pub const fn source_corner_triangles(self) -> &'a [[u32; 3]] {
        self.triangles
    }
    pub const fn source_face_triangle_offsets(self) -> &'a [u32] {
        self.face_offsets
    }
    pub const fn options_digest(self) -> OptionsDigest {
        self.options_digest
    }
}

#[derive(Debug, Error)]
pub enum SceneEditBootstrapError {
    #[error("the Scene-edit evidence envelope cannot be planned")]
    Plan(#[from] StorePlanError),
    #[error("a fresh Scene-edit request identity is unavailable")]
    RequestIdentityExhausted,
}

/// Startup facts independent of input inspection and of origin-buffer size.
///
/// Four success members are edit root, Scene counts, corner origins, and face offsets. Ordinary
/// rich failure keeps only the first two, with absent remap reference cells. The table root needs
/// one attribution slot in addition to these members. No final Scene generation is reserved here.
pub struct SceneEditBootstrap {
    limits: RetainedStoreBatchLimits,
    failure: CanonicalFailureRequirements,
    admission: SceneEditAdmission,
}

impl SceneEditBootstrap {
    pub fn try_new() -> Result<Self, SceneEditBootstrapError> {
        let (_, _, counts) = NodeSceneSessionActivationBootstrap::try_new()?.into_parts();
        let evidence = SceneEditFailureEvidence {
            root: RecordPlan::new(1, schema::ROOT_FIELDS)?,
            counts,
        };
        let limits = RetainedStoreBatchLimits::new(SUCCESS_EVIDENCE_MEMBERS + 1, 4)?;
        let (brand, request) = ConstructionBrand::reserve_with_activation_request()
            .map_err(|_| SceneEditBootstrapError::RequestIdentityExhausted)?;
        // The session-bound outcome table also holds the larger changed-evidence graph.
        let failure = CanonicalFailureRequirements::new(CanonicalTableShape::new(
            SUCCESS_EVIDENCE_MEMBERS,
            SUCCESS_EVIDENCE_MEMBERS,
        )?)
        .with_request_identity(brand.activation_request_identity());
        Ok(Self {
            limits,
            failure,
            admission: SceneEditAdmission {
                brand,
                request,
                evidence,
            },
        })
    }

    /// Pass limits and failure facts to `into_session_outcome_owner`; retain admission for the
    /// later transaction. The returned values all belong to the same opaque activation request.
    pub fn into_parts(
        self,
    ) -> (
        RetainedStoreBatchLimits,
        CanonicalFailureRequirements,
        SceneEditAdmission,
    ) {
        (self.limits, self.failure, self.admission)
    }

    pub const fn success_evidence_members() -> usize {
        SUCCESS_EVIDENCE_MEMBERS
    }
    pub const fn failure_evidence_members() -> usize {
        FAILURE_EVIDENCE_MEMBERS
    }
    pub const fn failure_field_work() -> u64 {
        FAILURE_FIELD_WORK
    }

    pub fn failure_terminal_work_requirement()
    -> Result<RetainedStoreFailureTerminalWorkRequirement, StorePlanError> {
        RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(
            FAILURE_EVIDENCE_MEMBERS,
            FAILURE_FIELD_WORK,
            FAILURE_FIELD_WORK,
        )
    }
}

/// One-use request half retained by postprocess while kernel scratch and origins are constructed.
pub struct SceneEditAdmission {
    brand: ConstructionBrand,
    request: CanonicalActivationRequest,
    evidence: SceneEditFailureEvidence,
}

impl SceneEditAdmission {
    pub const fn failure_evidence(&self) -> SceneEditFailureEvidence {
        self.evidence
    }

    /// No input-dependent reads, allocation, or publication occur until metered preflight.
    pub fn into_transaction<'a>(
        self,
        input: &'a ValidatedScene,
        replacement: SceneTriangulationReplacement<'a>,
        limits: SceneValidationLimits,
    ) -> SceneEditTransaction<'a> {
        self.transaction(input, EditRequest::Triangulation(replacement), limits)
    }

    pub fn into_normal_transaction<'a>(
        self,
        input: &'a ValidatedScene,
        replacement: SceneNormalReplacement<'a>,
        limits: SceneValidationLimits,
    ) -> SceneEditTransaction<'a> {
        self.transaction(input, EditRequest::Normals(replacement), limits)
    }

    pub fn into_space_transaction<'a>(
        self,
        resolved: ResolvedSceneSpaceConversion<'a>,
        limits: SceneValidationLimits,
    ) -> SceneEditTransaction<'a> {
        self.transaction(resolved.input, EditRequest::ConvertSpace(resolved), limits)
    }

    fn transaction<'a>(
        self,
        input: &'a ValidatedScene,
        replacement: EditRequest<'a>,
        limits: SceneValidationLimits,
    ) -> SceneEditTransaction<'a> {
        SceneEditTransaction {
            input,
            replacement,
            limits,
            evidence: self.evidence,
            brand: self.brand,
            storage: None,
            execution: Some(self),
        }
    }
}

/// The concrete two-member ordinary-failure recipe; no caller-supplied Scene counts are accepted.
#[derive(Clone, Copy)]
pub struct SceneEditFailureEvidence {
    root: RecordPlan,
    counts: NodeSceneEvidenceRecipe,
}

impl SceneEditFailureEvidence {
    pub const fn root_record_plan(&self) -> RecordPlan {
        self.root
    }
    pub const fn scene_counts(&self) -> &NodeSceneEvidenceRecipe {
        &self.counts
    }

    /// The caller first retains `scene_counts()` using `SceneOutcomeEvidenceTag`, then roots
    /// that exact member through this row using `SceneEditEvidenceTag`.
    pub fn write_root_row(
        &self,
        row: &mut OutcomeEvidenceRecordRowWriter<'_>,
        counts: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    ) -> Result<(), StoreBuildError> {
        row.write_scalar(schema::REVISION)?;
        row.write_scalar(schema::OUTCOME_FAILED)?;
        row.write_scalar(schema::OPERATION_SEMANTIC_VERSION)?;
        // No successful edit or options claim is made for a failed invocation.
        for _ in schema::field::OPERATION_KIND..schema::field::OPTIONS_DIGEST_FIRST {
            row.write_scalar(0_u32)?;
        }
        for _ in schema::field::OPTIONS_DIGEST_FIRST..schema::field::SCENE_COUNTS {
            row.write_scalar(0_u64)?;
        }
        row.write_record_ref(counts)?;
        row.write_scalar(0_u8)?;
        row.write_scalar(0_u8)
    }
}

/// Same-account start without exposing a physical account or canonical source-store capability.
pub fn try_start_scene_edit_operation<O, PP, U>(
    input: &ValidatedScene,
    control: OperationControl<PP, U>,
    ledger_plan: StoreLedgerPlan,
    start_plan: RetainedStoreStartPlan,
) -> Result<DerivedRetainedStoreOperation<O, PP, U>, RetainedStoreStartError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    RetainedStoreOperation::<O, PP, U>::try_start_derived(
        control,
        ledger_plan,
        &input.as_node_scene().canonical,
        start_plan,
    )
}

/// Source-bound corner normals and the explicit authored-field treatment.
#[derive(Clone, Copy)]
pub struct SceneNormalReplacement<'a> {
    target: PrimitiveId,
    normals: &'a [[f32; 3]],
    replace_existing: bool,
    remove_tangents: bool,
    smooth: bool,
    options_digest: OptionsDigest,
}

impl<'a> SceneNormalReplacement<'a> {
    pub const fn new(
        target: PrimitiveId,
        normals: &'a [[f32; 3]],
        replace_existing: bool,
        remove_tangents: bool,
        smooth: bool,
        options_digest: OptionsDigest,
    ) -> Self {
        Self {
            target,
            normals,
            replace_existing,
            remove_tangents,
            smooth,
            options_digest,
        }
    }
}

#[derive(Clone, Copy)]
enum EditRequest<'a> {
    Triangulation(SceneTriangulationReplacement<'a>),
    Normals(SceneNormalReplacement<'a>),
    ConvertSpace(ResolvedSceneSpaceConversion<'a>),
}

/// The real source domain addressed by a Scene edit; whole-scene edits have no primitive ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneEditTarget {
    Primitive(PrimitiveId),
    WholeScene(SceneGenerationId),
}

impl<'a> EditRequest<'a> {
    fn target(self) -> SceneEditTarget {
        match self {
            Self::Triangulation(v) => SceneEditTarget::Primitive(v.target),
            Self::Normals(v) => SceneEditTarget::Primitive(v.target),
            Self::ConvertSpace(v) => SceneEditTarget::WholeScene(v.input.generation()),
        }
    }
    const fn triangles(self) -> &'a [[u32; 3]] {
        match self {
            Self::Triangulation(v) => v.triangles,
            Self::Normals(_) | Self::ConvertSpace(_) => &[],
        }
    }
    const fn face_offsets(self) -> &'a [u32] {
        match self {
            Self::Triangulation(v) => v.face_offsets,
            Self::Normals(_) | Self::ConvertSpace(_) => &[],
        }
    }
}

#[derive(Clone, Copy)]
#[allow(
    clippy::large_enum_variant,
    reason = "bounded source-bound plan headers stay inline without another allocation; one-MiB-stack tests cover direct operations"
)]
enum EditStoragePlan<'a> {
    Triangulation(GeometryTriangulationPlan<'a>),
    Normals(GeometryNormalsPlan<'a>),
    ConvertSpace(SceneSpacePlan<'a>),
}

impl EditStoragePlan<'_> {
    fn correspondence_work(self) -> u64 {
        match self {
            Self::Triangulation(v) => v.correspondence_work(),
            Self::Normals(v) => v.correspondence_work(),
            Self::ConvertSpace(plan) => plan.correspondence_work(),
        }
    }
    fn name_bytes(self) -> usize {
        match self {
            Self::Triangulation(v) => v.name_bytes(),
            Self::Normals(v) => v.name_bytes(),
            Self::ConvertSpace(plan) => plan.name_bytes(),
        }
    }
    fn facts(
        self,
        input: &super::super::SceneStorage,
        replacement: EditRequest<'_>,
        counts: super::super::SceneEvidenceCounts,
    ) -> Option<evidence::SceneEditFacts> {
        match (self, replacement) {
            (Self::Triangulation(plan), EditRequest::Triangulation(replacement)) => {
                Some(evidence::SceneEditFacts::new(input, replacement, &plan))
            }
            (Self::Normals(plan), EditRequest::Normals(replacement)) => {
                evidence::SceneEditFacts::normals(input, replacement, &plan, counts)
            }
            (Self::ConvertSpace(plan), EditRequest::ConvertSpace(resolved)) => {
                (std::ptr::eq(input, resolved.input.as_node_scene())
                    && plan.matches_source(&input.tables)
                    && plan.conversion() == resolved.conversion)
                    .then(|| evidence::SceneEditFacts::space(resolved))
            }
            _ => None,
        }
    }
    fn validate_correspondence<
        C: crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl,
    >(
        self,
        actual: &crate::canonical_scene::store::typed_storage::SceneTables,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        match self {
            Self::Triangulation(plan) => {
                plan.validate_correspondence(actual.geometry.as_ref(), control)
            }
            Self::Normals(plan) => plan.validate_correspondence(actual.geometry.as_ref(), control),
            Self::ConvertSpace(plan) => plan.validate_correspondence(actual, control),
        }
    }
}

fn map_space(error: SceneSpacePlanError<StoreRuntimeError>) -> SceneEditRejection {
    match error {
        SceneSpacePlanError::Nodes(error) => map_node_space(error),
        SceneSpacePlanError::Geometry(error) => map_geometry_space(error),
        SceneSpacePlanError::Deformation(error) => map_deformation_space(error),
        SceneSpacePlanError::Animation(error) => map_animation_space(error),
        SceneSpacePlanError::Spatial(error) => map_spatial_space(error),
        SceneSpacePlanError::Plan(error) => StoreRuntimeError::Plan(error).into(),
    }
}

fn map_spatial_space(error: SpatialSpacePlanError<StoreRuntimeError>) -> SceneEditRejection {
    match error {
        SpatialSpacePlanError::Control(error) => error.into(),
        SpatialSpacePlanError::NumericCamera { camera, source } => {
            SceneSpaceConversionIssue::NumericCamera { camera, source }.into()
        }
        SpatialSpacePlanError::CameraProjection { camera, source } => {
            SceneSpaceConversionIssue::CameraProjection { camera, source }.into()
        }
        SpatialSpacePlanError::NumericLight { light, source } => {
            SceneSpaceConversionIssue::NumericLight { light, source }.into()
        }
        SpatialSpacePlanError::Plan(error) => StoreRuntimeError::Plan(error).into(),
        SpatialSpacePlanError::Invariant => SceneEditRejection::construction_invariant(),
    }
}

fn map_node_space(error: NodeSpacePlanError<StoreRuntimeError>) -> SceneEditRejection {
    match error {
        NodeSpacePlanError::Control(error) => error.into(),
        NodeSpacePlanError::Numeric { node, source } => {
            SceneSpaceConversionIssue::NumericNode { node, source }.into()
        }
        NodeSpacePlanError::Plan(error) => StoreRuntimeError::Plan(error).into(),
        NodeSpacePlanError::Invariant => SceneEditRejection::construction_invariant(),
    }
}

fn map_geometry_space(error: GeometrySpacePlanError<StoreRuntimeError>) -> SceneEditRejection {
    match error {
        GeometrySpacePlanError::Control(error) => error.into(),
        GeometrySpacePlanError::Numeric { attribute, source } => {
            SceneSpaceConversionIssue::NumericGeometry { attribute, source }.into()
        }
        GeometrySpacePlanError::UnsupportedTopology {
            primitive,
            topology,
        } => SceneSpaceConversionIssue::UnsupportedTopology {
            primitive,
            topology,
        }
        .into(),
        GeometrySpacePlanError::CustomAttribute { attribute } => {
            SceneSpaceConversionIssue::CustomAttribute { attribute }.into()
        }
        GeometrySpacePlanError::Plan(error) => StoreRuntimeError::Plan(error).into(),
        GeometrySpacePlanError::Invariant => SceneEditRejection::construction_invariant(),
    }
}

fn map_deformation_space(
    error: DeformationSpacePlanError<StoreRuntimeError>,
) -> SceneEditRejection {
    match error {
        DeformationSpacePlanError::Control(error) => error.into(),
        DeformationSpacePlanError::Numeric { location, source } => match location {
            DeformationSpaceNumericLocation::SkinCorrection { skin } => {
                SceneSpaceConversionIssue::NumericSkinCorrection { skin, source }
            }
            DeformationSpaceNumericLocation::InverseBind { matrix } => {
                SceneSpaceConversionIssue::NumericInverseBind { matrix, source }
            }
            DeformationSpaceNumericLocation::MorphDelta { delta, tuple } => {
                SceneSpaceConversionIssue::NumericMorphDelta {
                    delta,
                    tuple,
                    source,
                }
            }
        }
        .into(),
        DeformationSpacePlanError::Plan(error) => StoreRuntimeError::Plan(error).into(),
        DeformationSpacePlanError::Invariant => SceneEditRejection::construction_invariant(),
    }
}

fn map_animation_space(error: AnimationSpacePlanError<StoreRuntimeError>) -> SceneEditRejection {
    match error {
        AnimationSpacePlanError::Control(error) => error.into(),
        AnimationSpacePlanError::Numeric {
            channel,
            part,
            key,
            source,
        } => SceneSpaceConversionIssue::NumericAnimation {
            channel,
            part,
            key,
            source,
        }
        .into(),
        AnimationSpacePlanError::Plan(error) => StoreRuntimeError::Plan(error).into(),
        AnimationSpacePlanError::Invariant => SceneEditRejection::construction_invariant(),
    }
}

#[derive(Debug, Error)]
pub enum SceneEditRejection {
    #[error("space conversion is unsupported: {0}")]
    Space(#[from] SceneSpaceConversionIssue),
    #[error("the Scene-edit target or origin relation is invalid")]
    Compile(#[from] SceneEditCompileError),
    #[error("Scene-edit planning or storage admission failed")]
    Runtime(#[from] StoreRuntimeError),
    #[error("Scene-edit {resource} needs {actual} bytes, above its {maximum} byte limit")]
    Limit {
        resource: &'static str,
        actual: u64,
        maximum: u64,
    },
}

/// Single-use authority-free description; the host owns and resolves every semantic lane.
pub struct SceneEditTransaction<'a> {
    input: &'a ValidatedScene,
    replacement: EditRequest<'a>,
    limits: SceneValidationLimits,
    evidence: SceneEditFailureEvidence,
    execution: Option<SceneEditAdmission>,
    brand: ConstructionBrand,
    storage: Option<EditStoragePlan<'a>>,
}

/// Borrowed postflight description, independent of the operation and its scoped scratch owner.
///
/// The unchanged source and origin buffers stay alive until core has checked actual candidate
/// correspondence. Failure evidence remains available even when preflight rejected the request.
pub struct SceneEditPostflight<'a> {
    input: &'a ValidatedScene,
    replacement: EditRequest<'a>,
    limits: SceneValidationLimits,
    evidence: SceneEditFailureEvidence,
    brand: ConstructionBrand,
    storage: Option<EditStoragePlan<'a>>,
}

impl SceneEditPostflight<'_> {
    pub const fn failure_evidence(&self) -> SceneEditFailureEvidence {
        self.evidence
    }

    pub fn target(&self) -> SceneEditTarget {
        self.replacement.target()
    }

    pub const fn validation_limits(&self) -> SceneValidationLimits {
        self.limits
    }
}

/// All origin-dependent facts remain bound to the exact source through a borrow, not a raw ID.
pub struct SceneEditPlan<'a, 'scope, O: OperationOwner> {
    admission: SceneEditAdmission,
    source_generation: SceneGenerationId,
    replacement: EditRequest<'a>,
    sealed: Vec<BoundPendingNodeSceneEdit<'a>>,
    success_requirements: CanonicalSuccessRequirements,
    reservation: CanonicalReservation<'scope, O>,
}

impl<'a> SceneEditTransaction<'a> {
    /// Drops any unused activation request; the recipe borrows the exact source and origins.
    pub fn finish(self) -> SceneEditPostflight<'a> {
        SceneEditPostflight {
            input: self.input,
            replacement: self.replacement,
            limits: self.limits,
            evidence: self.evidence,
            brand: self.brand,
            storage: self.storage,
        }
    }

    fn require_validation_scratch(&self, actual: u64) -> Result<(), SceneEditRejection> {
        if actual > self.limits.max_scratch_bytes() {
            return Err(SceneEditRejection::Limit {
                resource: "central validation scratch",
                actual,
                maximum: self.limits.max_scratch_bytes(),
            });
        }
        Ok(())
    }

    pub fn preflight<'scope, O, PP, U>(
        &mut self,
        semantic: &mut CanonicalSemanticWorkSession<
            '_,
            'scope,
            O,
            PP,
            U,
            CanonicalInheritanceSource,
        >,
    ) -> Result<SceneEditPlan<'a, 'scope, O>, SceneEditRejection>
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        let admission = self
            .execution
            .take()
            .ok_or(SceneEditCompileError::InvalidSource)?;
        let capacity = CapacityPlan::for_type::<BoundPendingNodeSceneEdit<'a>>(0, 0, 1)
            .map_err(|cause| StoreRuntimeError::Plan(cause.into()))?;
        let bytes = capacity
            .envelope_delta_bytes()
            .map_err(|cause| StoreRuntimeError::Plan(cause.into()))?;
        if bytes > self.limits.max_scratch_bytes() {
            return Err(SceneEditRejection::Limit {
                resource: "sealed plan",
                actual: bytes,
                maximum: self.limits.max_scratch_bytes(),
            });
        }
        let mut reservation = semantic.reserve(bytes)?;
        let mut sealed = Vec::new();
        sealed
            .try_reserve_exact(capacity.allocator_additional())
            .map_err(|_| StoreRuntimeError::BatchAllocationFailed)?;
        let bytes = capacity
            .finish(sealed.len(), sealed.capacity())
            .map_err(|cause| StoreRuntimeError::Plan(cause.into()))?;
        semantic.resize(&mut reservation, bytes)?;
        let input = self.input;
        let target_row = if let SceneEditTarget::Primitive(target) = self.replacement.target() {
            Some(semantic.run_unit(|| {
                // Public lookup validates generation, logical-mesh ownership, and the local index.
                input
                    .primitive(target)
                    .map_err(|_| SceneEditCompileError::InvalidTarget)?;
                let mesh = input
                    .as_node_scene()
                    .read_geometry()
                    .logical_meshes
                    .get(target.logical_mesh().index() as usize)
                    .ok_or(SceneEditCompileError::InvalidTarget)?;
                let row = mesh
                    .primitives
                    .start()
                    .checked_add(target.local() as usize)
                    .ok_or(SceneEditCompileError::SizeOverflow)?;
                let primitive = input
                    .as_node_scene()
                    .read_geometry()
                    .primitives
                    .get(row)
                    .ok_or(SceneEditCompileError::InvalidTarget)?;
                let valid_topology = match self.replacement {
                    EditRequest::Triangulation(_) => primitive.topology == TopologyKind::Polygons,
                    EditRequest::Normals(_) => primitive.topology == TopologyKind::TriangleList,
                    EditRequest::ConvertSpace(_) => false,
                };
                if !valid_topology || primitive.face_count == 0 {
                    return Err(SceneEditCompileError::InvalidSource);
                }
                Ok(row)
            })??)
        } else {
            None
        };
        let (plan, pending) = match self.replacement {
            EditRequest::Triangulation(replacement) => {
                let plan = GeometryTriangulationPlan::try_new(
                    input.as_node_scene().read_geometry(),
                    target_row.ok_or(SceneEditCompileError::InvalidTarget)?,
                    TriangulationOrigins {
                        triangles: replacement.triangles,
                        face_offsets: replacement.face_offsets,
                    },
                    semantic,
                )
                .map_err(map_build)?;
                let pending = BoundPendingNodeSceneEdit::try_triangulation(
                    input.as_node_scene(),
                    plan,
                    replacement.options_digest,
                    &mut || {
                        semantic
                            .run_unit(|| ())
                            .map_err(NodeSceneEditPlanError::from)
                    },
                )
                .map_err(map_plan)?;
                (EditStoragePlan::Triangulation(plan), pending)
            }
            EditRequest::Normals(replacement) => {
                let plan = GeometryNormalsPlan::try_new(
                    input.as_node_scene().read_geometry(),
                    target_row.ok_or(SceneEditCompileError::InvalidTarget)?,
                    replacement.normals,
                    if replacement.replace_existing {
                        NormalWritePolicy::Replace
                    } else {
                        NormalWritePolicy::Add
                    },
                    if replacement.remove_tangents {
                        TangentPolicy::Remove
                    } else {
                        TangentPolicy::Reject
                    },
                    semantic,
                )
                .map_err(map_build)?;
                let pending = BoundPendingNodeSceneEdit::try_normals(
                    input.as_node_scene(),
                    plan,
                    replacement.smooth,
                    replacement.options_digest,
                    &mut || {
                        semantic
                            .run_unit(|| ())
                            .map_err(NodeSceneEditPlanError::from)
                    },
                )
                .map_err(map_plan)?;
                (EditStoragePlan::Normals(plan), pending)
            }
            EditRequest::ConvertSpace(resolved) => {
                space::check_capabilities(resolved, semantic)?;
                let plan = SceneSpacePlan::try_new(
                    &input.as_node_scene().tables,
                    resolved.conversion,
                    semantic,
                )
                .map_err(map_space)?;
                let pending = BoundPendingNodeSceneEdit::try_space_conversion(
                    input.as_node_scene(),
                    plan,
                    resolved.options_digest(),
                    &mut || {
                        semantic
                            .run_unit(|| ())
                            .map_err(NodeSceneEditPlanError::from)
                    },
                )
                .map_err(map_plan)?;
                (EditStoragePlan::ConvertSpace(plan), pending)
            }
        };
        if u64::from(pending.counts.profile.maximum_depth) > self.limits.max_provenance_depth() {
            return Err(SceneEditRejection::Limit {
                resource: "provenance depth",
                actual: u64::from(pending.counts.profile.maximum_depth),
                maximum: self.limits.max_provenance_depth(),
            });
        }
        let validation_scratch = pending
            .counts
            .central_validation_plan
            .scratch_plan()
            .envelope_bytes();
        self.require_validation_scratch(validation_scratch)?;
        let total = pending.counts.canonical_blocks.as_usize();
        let extra_work = (schema::ROOT_FIELDS as u64)
            // The rebound Scene plan already includes its Scene-count evidence record.
            .checked_add(
                (self.replacement.triangles().len() as u64)
                    .checked_mul(3)
                    .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?,
            )
            .and_then(|n| n.checked_add(self.replacement.face_offsets().len() as u64))
            .and_then(|n| n.checked_add(plan.correspondence_work()))
            .and_then(|n| n.checked_add(pending.counts.central_validation_plan.work().get()))
            .and_then(std::num::NonZeroU64::new)
            .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        if extra_work.get() > self.limits.max_work() {
            return Err(SceneEditRejection::Limit {
                resource: "central validation work",
                actual: extra_work.get(),
                maximum: self.limits.max_work(),
            });
        }
        let success_requirements = CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(total, total).map_err(StoreRuntimeError::Plan)?,
            4,
            total,
            extra_work,
        )
        .map_err(StoreRuntimeError::Plan)?
        .with_request_identity(admission.brand.activation_request_identity())
        .with_semantic_bytes(plan.name_bytes() as u64);
        self.storage = Some(plan);
        sealed.push(pending);
        Ok(SceneEditPlan {
            admission,
            source_generation: input.generation(),
            replacement: self.replacement,
            sealed,
            success_requirements,
            reservation,
        })
    }
}

impl<O, PP, U> TypedStorageBuildControl
    for CanonicalSemanticWorkSession<'_, '_, O, PP, U, CanonicalInheritanceSource>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    type Error = StoreRuntimeError;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        let mut action = Some(action);
        CanonicalSemanticWorkSession::run_unit(self, || {
            action.take().expect("a semantic action is executed once")()
        })
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        TypedStorageBuildControl::run_unit(self, action)
    }
}

impl<O: OperationOwner> SceneEditPlan<'_, '_, O> {
    pub fn target(&self) -> SceneEditTarget {
        self.replacement.target()
    }
    pub const fn input_generation(&self) -> SceneGenerationId {
        self.source_generation
    }
}

/// Execution bound to the exact admitted request and its private source-derived plans.
pub struct SceneEditExecutionPlan<'a, 'scope, O: OperationOwner> {
    brand: ConstructionBrand,
    replacement: EditRequest<'a>,
    sealed: Vec<BoundPendingNodeSceneEdit<'a>>,
    _reservation: CanonicalReservation<'scope, O>,
}

impl<'a, 'scope, O: OperationOwner> CanonicalActivationDescription
    for SceneEditPlan<'a, 'scope, O>
{
    type Execution = SceneEditExecutionPlan<'a, 'scope, O>;
    fn seal_activation<'activation>(
        self,
        sealer: CanonicalActivationPlanSealer<'activation>,
    ) -> CanonicalSealedActivationPlan<'activation, Self::Execution> {
        let Self {
            admission,
            replacement,
            sealed,
            success_requirements,
            reservation,
            ..
        } = self;
        sealer.seal(
            admission.request,
            success_requirements,
            SceneEditExecutionPlan {
                brand: admission.brand,
                replacement,
                sealed,
                _reservation: reservation,
            },
        )
    }
}

/// Actual canonical output, carrying source-bound edit facts but no source Scene owner.
pub struct SceneEditOutput {
    scene: NodeSceneLaneOutput,
    facts: evidence::SceneEditFacts,
    source_generation: SceneGenerationId,
}

impl<O: OperationOwner> SceneEditExecutionPlan<'_, '_, O> {
    pub fn materialize<'lane, PP: ProgressPhase, U: ProgressUnit>(
        &self,
        mut lane: CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    ) -> CanonicalMaterialization<
        'lane,
        SceneEditOutput,
        SceneEditRejection,
        RecordBlock,
        SceneDirectorySchema,
    > {
        let pending = &self.sealed[0];
        let plan = match &pending.program {
            NodeSceneEditProgram::TriangulatePrimitive(geometry) => {
                EditStoragePlan::Triangulation(*geometry)
            }
            NodeSceneEditProgram::GenerateNormals { geometry, .. } => {
                EditStoragePlan::Normals(*geometry)
            }
            NodeSceneEditProgram::ConvertSpace(plan) => EditStoragePlan::ConvertSpace(*plan),
            _ => {
                let rejection = SceneEditRejection::construction_invariant();
                let code = rejection.stable_code();
                return lane.reject(rejection, code);
            }
        };
        let Some(facts) = plan.facts(pending.input, self.replacement, pending.counts.evidence)
        else {
            let rejection = SceneEditRejection::construction_invariant();
            let code = rejection.stable_code();
            return lane.reject(rejection, code);
        };
        let graph = match super::graph::try_rewrite_node_scene(&mut lane, pending) {
            Ok(graph) => graph,
            Err(cause) => {
                let rejection = SceneEditRejection::Runtime(cause);
                let code = rejection.stable_code();
                return lane.reject(rejection, code);
            }
        };
        let directory = graph.directory;
        let output = SceneEditOutput {
            scene: NodeSceneLaneOutput::from_inner(NodeSceneActivationOutput {
                tables: graph.tables,
                draft: NodeSceneDraft {
                    brand: self.brand,
                    directory,
                },
                refs: graph.refs,
                evidence_counts: pending.counts.evidence,
                central_validation: pending.counts.central_validation_plan,
            }),
            facts,
            source_generation: pending.input.generation(),
        };
        lane.complete(directory.into_store(), output)
    }
}

impl SceneEditRejection {
    pub const fn stable_code(&self) -> u32 {
        0x5343_1201
    }
    pub fn construction_invariant() -> Self {
        Self::Runtime(StoreRuntimeError::ConstructionInvariantUnverified)
    }
    pub fn control_cause(&self) -> Option<&baozi_operation::ControlCause> {
        match self {
            Self::Runtime(StoreRuntimeError::Control(cause)) => Some(cause),
            _ => None,
        }
    }
}

fn map_plan(cause: NodeSceneEditPlanError) -> SceneEditRejection {
    match cause {
        NodeSceneEditPlanError::Runtime(cause) => cause.into(),
        NodeSceneEditPlanError::ValidationPlan(cause) => StoreRuntimeError::Plan(cause).into(),
        NodeSceneEditPlanError::SizeOverflow { .. } => SceneEditCompileError::SizeOverflow.into(),
        _ => SceneEditCompileError::InvalidSource.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::tests::{test_account, test_factory};
    use super::super::tests::{build_populated_spatial_scene, build_single_trs_scene};
    use super::*;
    use crate::scene_space::{Axis, FrontFaceWinding, Handedness, SceneSpaceField};
    use crate::space_conversion::{SourceSpaceError, TargetSpace};
    use std::sync::atomic::AtomicU64;

    #[derive(Default)]
    struct Control(u64);

    impl TypedStorageBuildControl for Control {
        type Error = StoreRuntimeError;
        fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
            self.0 += 1;
            Ok(action())
        }
        fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
            self.run_unit(action)
        }
    }

    impl crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl
        for Control
    {
        type Error = StoreRuntimeError;
        fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, Self::Error> {
            TypedStorageBuildControl::run_unit(self, action).map(Some)
        }
    }

    #[test]
    fn startup_evidence_shape_is_fixed_and_bootstrap_does_not_allocate() {
        let mut bootstrap = None;
        let allocations = allocation_counter::measure(|| {
            bootstrap = Some(
                SceneEditBootstrap::try_new()
                    .expect("fixed edit bootstrap must plan without a source Scene"),
            );
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
        let (writer, _failure, admission) = bootstrap.unwrap().into_parts();
        assert_eq!(writer.attribution_slots(), 5);
        assert_eq!(SceneEditBootstrap::success_evidence_members(), 4);
        assert_eq!(SceneEditBootstrap::failure_evidence_members(), 2);
        let evidence = admission.failure_evidence();
        assert_eq!(evidence.root_record_plan().rows(), 1);
        assert_eq!(evidence.root_record_plan().fields(), 25);
        assert_eq!(evidence.scene_counts().record_plan().rows(), 1);
        assert_eq!(evidence.scene_counts().record_plan().fields(), 34);
        assert_eq!(SceneEditBootstrap::failure_field_work(), 59);
    }

    #[test]
    fn resolved_request_and_postflight_keep_the_exact_same_generation_source_borrow() {
        let factory = test_factory();
        let account = test_account(&factory, 1_024);
        let first = build_populated_spatial_scene(&account, &AtomicU64::new(1));
        let second = build_populated_spatial_scene(&account, &AtomicU64::new(1));
        let first_scene = ValidatedScene::from_node_scene(first.scene);
        let second_scene = ValidatedScene::from_node_scene(second.scene);
        assert_eq!(first_scene.generation(), second_scene.generation());
        assert_ne!(
            first_scene.as_node_scene().proof.directory(),
            second_scene.as_node_scene().proof.directory()
        );
        let mut resolved = None;
        let allocation = allocation_counter::measure(|| {
            let request =
                resolve_scene_space_conversion(&first_scene, TargetSpace::STANDARD, None).unwrap();
            assert!(request.is_unchanged());
            let _ = request.options_digest();
            resolved = Some(request);
        });
        assert_eq!(allocation.count_total, 0);
        let (_, _, admission) = SceneEditBootstrap::try_new().unwrap().into_parts();
        let transaction =
            admission.into_space_transaction(resolved.unwrap(), SceneValidationLimits::default());
        assert!(std::ptr::eq(transaction.input, &first_scene));
        assert!(!std::ptr::eq(transaction.input, &second_scene));
        let postflight = transaction.finish();
        assert!(std::ptr::eq(postflight.input, &first_scene));
        assert_eq!(
            postflight.target(),
            SceneEditTarget::WholeScene(first_scene.generation())
        );
        let mut control = Control::default();
        let storage = SceneSpacePlan::try_new(
            &first_scene.as_node_scene().tables,
            resolved.unwrap().conversion,
            &mut control,
        )
        .unwrap();
        assert!(storage.matches_source(&first_scene.as_node_scene().tables));
        assert!(!storage.matches_source(&second_scene.as_node_scene().tables));
        control.0 = 0;
        assert!(
            storage
                .spatial()
                .validate_correspondence(
                    first_scene.as_node_scene().tables.spatial.as_ref(),
                    &mut control,
                )
                .unwrap()
        );
        assert_eq!(
            control.0, 3,
            "one bounded space action and one action per actual definition"
        );
        drop(first_scene);
        drop(second_scene);
        drop(first.report);
        drop(second.report);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn explicit_source_assumptions_are_a_changed_declaration_with_a_fixed_options_digest() {
        let factory = test_factory();
        let account = test_account(&factory, 1_024);
        let source = build_single_trs_scene(&account, &AtomicU64::new(1));
        let scene = ValidatedScene::from_node_scene(source.scene);
        assert!(matches!(
            resolve_scene_space_conversion(&scene, TargetSpace::STANDARD, None),
            Err(SourceSpaceError::MissingSourceConvention {
                field: SceneSpaceField::Handedness
            })
        ));
        let resolved = resolve_scene_space_conversion(
            &scene,
            TargetSpace::STANDARD,
            Some(TargetSpace::STANDARD),
        )
        .unwrap();
        assert!(!resolved.is_unchanged());
        assert!(!resolved.changes_coordinates());
        assert!(!resolved.reverses_winding());
        assert_eq!(resolved.source(), TargetSpace::STANDARD);
        assert_eq!(resolved.target(), TargetSpace::STANDARD);
        for field in [
            SceneSpaceField::Handedness,
            SceneSpaceField::UpAxis,
            SceneSpaceField::FrontAxis,
            SceneSpaceField::UnitScaleToMeters,
            SceneSpaceField::FrontFaceWinding,
        ] {
            assert!(resolved.assumed_source_field(field));
        }
        assert!(!resolved.assumed_source_field(SceneSpaceField::UvOrigin));
        // Independently encoded fixed domain, LE revision=1, twice [1,3,6,2]+f64(1), mask=31.
        assert_eq!(
            *resolved.options_digest().as_bytes(),
            [
                0x96, 0x29, 0xcc, 0xdd, 0x92, 0x8c, 0x66, 0x42, 0xcc, 0x43, 0x87, 0x33, 0x47, 0x88,
                0x14, 0xa4, 0x88, 0x4d, 0x91, 0xf8, 0x64, 0xbf, 0x8a, 0x36, 0x86, 0x53, 0x12, 0x33,
                0x29, 0xc9, 0xeb, 0x2a,
            ]
        );
        let facts = evidence::SceneEditFacts::space(resolved);
        assert_eq!(facts.units, [1.0_f64.to_bits(); 2]);
        assert_eq!(
            facts.words,
            [
                schema::OPERATION_SPACE_CONVERSION,
                1,
                3,
                6,
                2,
                1,
                3,
                6,
                2,
                31,
                0,
                0,
                0
            ]
        );
        drop(scene);
        drop(source.report);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
    }

    #[test]
    fn rebound_validation_scratch_accepts_exact_limit_and_rejects_one_byte_less() {
        let factory = test_factory();
        let account = test_account(&factory, 1_024);
        let source = build_single_trs_scene(&account, &AtomicU64::new(1));
        let scene = ValidatedScene::from_node_scene(source.scene);
        let resolved = resolve_scene_space_conversion(
            &scene,
            TargetSpace::STANDARD,
            Some(TargetSpace::STANDARD),
        )
        .unwrap();
        assert!(!resolved.is_unchanged());
        let input = scene.as_node_scene();
        let mut control = Control::default();
        let plan =
            SceneSpacePlan::try_new(&input.tables, resolved.conversion, &mut control).unwrap();
        let pending = BoundPendingNodeSceneEdit::try_space_conversion(
            input,
            plan,
            resolved.options_digest(),
            &mut || Ok(()),
        )
        .unwrap();
        let source_bytes = input
            .proof
            .central_validation_plan()
            .scratch_plan()
            .envelope_bytes();
        let required = pending
            .counts
            .central_validation_plan
            .scratch_plan()
            .envelope_bytes();
        assert!(
            pending.counts.evidence.profile_provenance_count
                > input.proof.counts().profile_provenance_count
        );
        assert!(required > source_bytes);
        let one_below = required - 1;
        assert!(one_below >= source_bytes);

        for maximum in [required, one_below] {
            let (_, _, admission) = SceneEditBootstrap::try_new().unwrap().into_parts();
            let transaction = admission.into_space_transaction(
                resolved,
                SceneValidationLimits::default().with_max_scratch_bytes(maximum),
            );
            let result = transaction.require_validation_scratch(required);
            if maximum == required {
                result.expect("the rebound validation scratch fits its exact byte limit");
            } else {
                assert!(matches!(
                    result,
                    Err(SceneEditRejection::Limit {
                        resource: "central validation scratch",
                        actual,
                        maximum,
                    }) if actual == required && maximum == one_below
                ));
            }
        }

        drop(scene);
        drop(source.report);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn numeric_conversion_admits_camera_frames_and_winding_only_preserves_values() {
        let factory = test_factory();
        let account = test_account(&factory, 1_024);
        let source = build_populated_spatial_scene(&account, &AtomicU64::new(1));
        let scene = ValidatedScene::from_node_scene(source.scene);
        let scale = TargetSpace::try_new(
            Handedness::Right,
            Axis::PositiveY,
            Axis::NegativeZ,
            0.01,
            FrontFaceWinding::CounterClockwise,
        )
        .unwrap();
        let numeric = resolve_scene_space_conversion(&scene, scale, None).unwrap();
        space::check_capabilities(numeric, &mut Control::default()).unwrap();
        let plan = SceneSpacePlan::try_new(
            &scene.as_node_scene().tables,
            numeric.conversion,
            &mut Control::default(),
        )
        .unwrap();
        assert!(
            !plan
                .spatial()
                .validate_correspondence(
                    scene.as_node_scene().tables.spatial.as_ref(),
                    &mut Control::default(),
                )
                .unwrap(),
            "valid input data does not prove scaled output"
        );
        let winding = TargetSpace::try_new(
            Handedness::Right,
            Axis::PositiveY,
            Axis::NegativeZ,
            1.0,
            FrontFaceWinding::Clockwise,
        )
        .unwrap();
        let winding = resolve_scene_space_conversion(&scene, winding, None).unwrap();
        assert!(!winding.changes_coordinates());
        assert!(winding.reverses_winding());
        space::check_capabilities(winding, &mut Control::default()).unwrap();
        let plan = SceneSpacePlan::try_new(
            &scene.as_node_scene().tables,
            winding.conversion,
            &mut Control::default(),
        )
        .unwrap();
        assert!(
            !plan
                .spatial()
                .validate_correspondence(
                    scene.as_node_scene().tables.spatial.as_ref(),
                    &mut Control::default(),
                )
                .unwrap(),
            "valid storage without the target declaration is not a converted Scene"
        );
        drop(scene);
        drop(source.report);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
    }
}
