//! Concrete geometry-edit bootstrap and source-bound semantic transaction.
//!
//! This module introduces no operation owner: postprocess starts the derived runtime and drives the semantic/materialization
//! lanes. Existing graph, profile, central validation, and publication wiring will consume its plan.

mod evidence;
mod publication;
mod schema;
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
pub use evidence::SceneGeometryEditEvidenceRecipe;
pub use publication::*;
pub use validation::SceneGeometryEditValidationProof;

pub use schema::{
    SceneGeometryCornerOriginsTag, SceneGeometryEditEvidenceTag, SceneGeometryFaceOffsetsTag,
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
pub enum SceneGeometryEditBootstrapError {
    #[error("the geometry-edit evidence envelope cannot be planned")]
    Plan(#[from] StorePlanError),
    #[error("a fresh geometry-edit request identity is unavailable")]
    RequestIdentityExhausted,
}

/// Startup facts independent of input inspection and of origin-buffer size.
///
/// Four success members are edit root, Scene counts, corner origins, and face offsets. Ordinary
/// rich failure keeps only the first two, with absent remap reference cells. The table root needs
/// one attribution slot in addition to these members. No final Scene generation is reserved here.
pub struct SceneGeometryEditBootstrap {
    limits: RetainedStoreBatchLimits,
    failure: CanonicalFailureRequirements,
    admission: SceneGeometryEditAdmission,
}

impl SceneGeometryEditBootstrap {
    pub fn try_new() -> Result<Self, SceneGeometryEditBootstrapError> {
        let (_, _, counts) = NodeSceneSessionActivationBootstrap::try_new()?.into_parts();
        let evidence = SceneGeometryEditFailureEvidence {
            root: RecordPlan::new(1, schema::ROOT_FIELDS)?,
            counts,
        };
        let limits = RetainedStoreBatchLimits::new(SUCCESS_EVIDENCE_MEMBERS + 1, 4)?;
        let (brand, request) = ConstructionBrand::reserve_with_activation_request()
            .map_err(|_| SceneGeometryEditBootstrapError::RequestIdentityExhausted)?;
        // The session-bound outcome table also holds the larger changed-evidence graph.
        let failure = CanonicalFailureRequirements::new(CanonicalTableShape::new(
            SUCCESS_EVIDENCE_MEMBERS,
            SUCCESS_EVIDENCE_MEMBERS,
        )?)
        .with_request_identity(brand.activation_request_identity());
        Ok(Self {
            limits,
            failure,
            admission: SceneGeometryEditAdmission {
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
        SceneGeometryEditAdmission,
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
pub struct SceneGeometryEditAdmission {
    brand: ConstructionBrand,
    request: CanonicalActivationRequest,
    evidence: SceneGeometryEditFailureEvidence,
}

impl SceneGeometryEditAdmission {
    pub const fn failure_evidence(&self) -> SceneGeometryEditFailureEvidence {
        self.evidence
    }

    /// No input-dependent reads, allocation, or publication occur until metered preflight.
    pub fn into_transaction<'a>(
        self,
        input: &'a ValidatedScene,
        replacement: SceneTriangulationReplacement<'a>,
        limits: SceneValidationLimits,
    ) -> SceneGeometryEditTransaction<'a> {
        SceneGeometryEditTransaction {
            input,
            replacement,
            limits,
            evidence: self.evidence,
            brand: self.brand,
            geometry: None,
            execution: Some(self),
        }
    }
}

/// The concrete two-member ordinary-failure recipe; no caller-supplied Scene counts are accepted.
#[derive(Clone, Copy)]
pub struct SceneGeometryEditFailureEvidence {
    root: RecordPlan,
    counts: NodeSceneEvidenceRecipe,
}

impl SceneGeometryEditFailureEvidence {
    pub const fn root_record_plan(&self) -> RecordPlan {
        self.root
    }
    pub const fn scene_counts(&self) -> &NodeSceneEvidenceRecipe {
        &self.counts
    }

    /// The caller first retains `scene_counts()` using `SceneOutcomeEvidenceTag`, then roots
    /// that exact member through this row using `SceneGeometryEditEvidenceTag`.
    pub fn write_root_row(
        &self,
        row: &mut OutcomeEvidenceRecordRowWriter<'_>,
        counts: OutcomeEvidenceRecordRef<NodeSceneOutcomeEvidenceSchema>,
    ) -> Result<(), StoreBuildError> {
        row.write_scalar(schema::REVISION)?;
        row.write_scalar(schema::OUTCOME_FAILED)?;
        row.write_scalar(schema::TRIANGULATION_SEMANTIC_VERSION)?;
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

#[derive(Debug, Error)]
pub enum SceneGeometryEditRejection {
    #[error("the geometry-edit target or origin relation is invalid")]
    Compile(#[from] SceneEditCompileError),
    #[error("geometry-edit planning or storage admission failed")]
    Runtime(#[from] StoreRuntimeError),
    #[error("geometry-edit {resource} needs {actual} bytes, above its {maximum} byte limit")]
    Limit {
        resource: &'static str,
        actual: u64,
        maximum: u64,
    },
}

/// Single-use authority-free description; the host owns and resolves every semantic lane.
pub struct SceneGeometryEditTransaction<'a> {
    input: &'a ValidatedScene,
    replacement: SceneTriangulationReplacement<'a>,
    limits: SceneValidationLimits,
    evidence: SceneGeometryEditFailureEvidence,
    execution: Option<SceneGeometryEditAdmission>,
    brand: ConstructionBrand,
    geometry: Option<GeometryTriangulationPlan<'a>>,
}

/// Borrowed postflight description, independent of the operation and its scoped scratch owner.
///
/// The unchanged source and origin buffers stay alive until core has checked actual candidate
/// correspondence. Failure evidence remains available even when preflight rejected the request.
pub struct SceneGeometryEditPostflight<'a> {
    input: &'a ValidatedScene,
    replacement: SceneTriangulationReplacement<'a>,
    limits: SceneValidationLimits,
    evidence: SceneGeometryEditFailureEvidence,
    brand: ConstructionBrand,
    geometry: Option<GeometryTriangulationPlan<'a>>,
}

impl SceneGeometryEditPostflight<'_> {
    pub const fn failure_evidence(&self) -> SceneGeometryEditFailureEvidence {
        self.evidence
    }

    pub const fn target(&self) -> PrimitiveId {
        self.replacement.target
    }

    pub const fn validation_limits(&self) -> SceneValidationLimits {
        self.limits
    }
}

/// All origin-dependent facts remain bound to the exact source through a borrow, not a raw ID.
pub struct SceneGeometryEditPlan<'a, 'scope, O: OperationOwner> {
    admission: SceneGeometryEditAdmission,
    source_generation: SceneGenerationId,
    replacement: SceneTriangulationReplacement<'a>,
    sealed: Vec<BoundPendingNodeSceneEdit<'a>>,
    success_requirements: CanonicalSuccessRequirements,
    reservation: CanonicalReservation<'scope, O>,
}

impl<'a> SceneGeometryEditTransaction<'a> {
    /// Drops any unused activation request; the recipe borrows the exact source and origins.
    pub fn finish(self) -> SceneGeometryEditPostflight<'a> {
        SceneGeometryEditPostflight {
            input: self.input,
            replacement: self.replacement,
            limits: self.limits,
            evidence: self.evidence,
            brand: self.brand,
            geometry: self.geometry,
        }
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
    ) -> Result<SceneGeometryEditPlan<'a, 'scope, O>, SceneGeometryEditRejection>
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
            return Err(SceneGeometryEditRejection::Limit {
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
        let target = self.replacement.target;
        let input = self.input;
        let target_row = semantic.run_unit(|| {
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
            if primitive.topology != TopologyKind::Polygons || primitive.face_count == 0 {
                return Err(SceneEditCompileError::InvalidSource);
            }
            Ok(row)
        })??;
        let plan = GeometryTriangulationPlan::try_new(
            input.as_node_scene().read_geometry(),
            target_row,
            TriangulationOrigins {
                triangles: self.replacement.triangles,
                face_offsets: self.replacement.face_offsets,
            },
            semantic,
        )
        .map_err(map_build)?;
        let pending = BoundPendingNodeSceneEdit::try_triangulation(
            input.as_node_scene(),
            plan,
            self.replacement.options_digest,
            &mut || {
                semantic
                    .run_unit(|| ())
                    .map_err(NodeSceneEditPlanError::from)
            },
        )
        .map_err(map_plan)?;
        if u64::from(pending.counts.profile.maximum_depth) > self.limits.max_provenance_depth() {
            return Err(SceneGeometryEditRejection::Limit {
                resource: "provenance depth",
                actual: u64::from(pending.counts.profile.maximum_depth),
                maximum: self.limits.max_provenance_depth(),
            });
        }
        let total = pending.counts.canonical_blocks.as_usize();
        let extra_work = (schema::ROOT_FIELDS as u64)
            // The rebound Scene plan already includes its Scene-count evidence record.
            .checked_add(plan.output_corner_count() as u64)
            .and_then(|n| n.checked_add(self.replacement.face_offsets.len() as u64))
            .and_then(|n| n.checked_add(plan.correspondence_work()))
            .and_then(|n| n.checked_add(pending.counts.central_validation_plan.work().get()))
            .and_then(std::num::NonZeroU64::new)
            .ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        if extra_work.get() > self.limits.max_work() {
            return Err(SceneGeometryEditRejection::Limit {
                resource: "central validation work",
                actual: extra_work.get(),
                maximum: self.limits.max_work(),
            });
        }
        let success_requirements = CanonicalSuccessRequirements::new(
            CanonicalTableShape::new(total, pending.counts.new_canonical_blocks.as_usize())
                .map_err(StoreRuntimeError::Plan)?,
            4,
            total,
            extra_work,
        )
        .map_err(StoreRuntimeError::Plan)?
        .with_request_identity(admission.brand.activation_request_identity())
        .with_semantic_bytes(plan.name_bytes() as u64);
        self.geometry = Some(plan);
        sealed.push(pending);
        Ok(SceneGeometryEditPlan {
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

impl<O: OperationOwner> SceneGeometryEditPlan<'_, '_, O> {
    pub const fn target(&self) -> PrimitiveId {
        self.replacement.target
    }
    pub const fn input_generation(&self) -> SceneGenerationId {
        self.source_generation
    }
}

/// Execution bound to the exact admitted request and its private source-derived plans.
pub struct SceneGeometryEditExecutionPlan<'a, 'scope, O: OperationOwner> {
    brand: ConstructionBrand,
    replacement: SceneTriangulationReplacement<'a>,
    sealed: Vec<BoundPendingNodeSceneEdit<'a>>,
    _reservation: CanonicalReservation<'scope, O>,
}

impl<'a, 'scope, O: OperationOwner> CanonicalActivationDescription
    for SceneGeometryEditPlan<'a, 'scope, O>
{
    type Execution = SceneGeometryEditExecutionPlan<'a, 'scope, O>;
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
            SceneGeometryEditExecutionPlan {
                brand: admission.brand,
                replacement,
                sealed,
                _reservation: reservation,
            },
        )
    }
}

/// Actual canonical output, carrying source-bound edit facts but no source Scene owner.
pub struct SceneGeometryEditOutput {
    scene: NodeSceneLaneOutput,
    facts: evidence::GeometryEditFacts,
    source_generation: SceneGenerationId,
}

impl<O: OperationOwner> SceneGeometryEditExecutionPlan<'_, '_, O> {
    pub fn materialize<'lane, PP: ProgressPhase, U: ProgressUnit>(
        &self,
        mut lane: CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    ) -> CanonicalMaterialization<
        'lane,
        SceneGeometryEditOutput,
        SceneGeometryEditRejection,
        RecordBlock,
        SceneDirectorySchema,
    > {
        let pending = &self.sealed[0];
        let NodeSceneEditProgram::TriangulatePrimitive(geometry) = &pending.program else {
            let rejection = SceneGeometryEditRejection::construction_invariant();
            let code = rejection.stable_code();
            return lane.reject(rejection, code);
        };
        let graph = match super::graph::try_rewrite_node_scene(&mut lane, pending) {
            Ok(graph) => graph,
            Err(cause) => {
                let rejection = SceneGeometryEditRejection::Runtime(cause);
                let code = rejection.stable_code();
                return lane.reject(rejection, code);
            }
        };
        let facts = evidence::GeometryEditFacts::new(pending.input, self.replacement, geometry);
        let directory = graph.directory;
        let output = SceneGeometryEditOutput {
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

impl SceneGeometryEditRejection {
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

fn map_plan(cause: NodeSceneEditPlanError) -> SceneGeometryEditRejection {
    match cause {
        NodeSceneEditPlanError::Runtime(cause) => cause.into(),
        NodeSceneEditPlanError::ValidationPlan(cause) => StoreRuntimeError::Plan(cause).into(),
        NodeSceneEditPlanError::SizeOverflow { .. } => SceneEditCompileError::SizeOverflow.into(),
        _ => SceneEditCompileError::InvalidSource.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_evidence_shape_is_fixed_and_bootstrap_does_not_allocate() {
        let mut bootstrap = None;
        let allocations = allocation_counter::measure(|| {
            bootstrap = Some(
                SceneGeometryEditBootstrap::try_new()
                    .expect("fixed edit bootstrap must plan without a source Scene"),
            );
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
        let (writer, _failure, admission) = bootstrap.unwrap().into_parts();
        assert_eq!(writer.attribution_slots(), 5);
        assert_eq!(SceneGeometryEditBootstrap::success_evidence_members(), 4);
        assert_eq!(SceneGeometryEditBootstrap::failure_evidence_members(), 2);
        let evidence = admission.failure_evidence();
        assert_eq!(evidence.root_record_plan().rows(), 1);
        assert_eq!(evidence.root_record_plan().fields(), 23);
        assert_eq!(evidence.scene_counts().record_plan().rows(), 1);
        assert_eq!(evidence.scene_counts().record_plan().fields(), 34);
        assert_eq!(SceneGeometryEditBootstrap::failure_field_work(), 57);
    }
}
