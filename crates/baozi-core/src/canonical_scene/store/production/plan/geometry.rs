#![allow(
    clippy::result_large_err,
    reason = "geometry authoring failures retain bounded control evidence inline without a failure-path allocation"
)]

use std::error::Error;
use std::fmt;
use std::ops::Range;

use baozi_operation::{
    CapacityPlan, CapacityPlanError, OperationOwner, ProgressPhase, ProgressUnit,
    RetainedStoreReservation,
};
use thiserror::Error;

use crate::attribute::{AttributeDomain, AttributeSemantic, AttributeSemanticKind};
use crate::geometry::TopologyKind;
use crate::{
    SCENE_VALIDATION_POLL_INTERVAL, SceneReservationKind, SceneValidationControl,
    SceneValidationLimits, SceneValidationWork,
};

use super::super::control::{OperationAuthoringControl, OperationAuthoringControlError};
use super::super::input::Presence;
use super::super::input::appearance::MaterialLocalId;
use super::super::input::geometry::{
    AuthoringRange, GeometryAttributeAuthoring, GeometryAttributePayloadAuthoring,
    GeometryAuthoringBatch, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
};

const LOGICAL_U8_BYTES: u64 = 1;
const LOGICAL_U32_BYTES: u64 = 4;
const LOGICAL_U64_BYTES: u64 = 8;
const LOGICAL_F32_BYTES: u64 = 4;
const LOGICAL_REFERENCE_BYTES: u64 = 4;

const GEOMETRY_DIRECTORY_FIXED_BYTES: u64 = 5 * LOGICAL_U32_BYTES;
const LOGICAL_MESH_ROW_BYTES: u64 = 3 * LOGICAL_U32_BYTES + LOGICAL_REFERENCE_BYTES;
const MESH_INSTANCE_ROW_BYTES: u64 = 3 * LOGICAL_U32_BYTES + 2 * LOGICAL_REFERENCE_BYTES;
const PRIMITIVE_FIXED_BYTES: u64 = 9 * LOGICAL_U32_BYTES + 4 * LOGICAL_U8_BYTES;
const ATTRIBUTE_FIXED_BYTES: u64 =
    (4 * LOGICAL_U32_BYTES) + LOGICAL_U64_BYTES + (6 * LOGICAL_U8_BYTES);

pub(in crate::canonical_scene::store) fn geometry_shape_scratch_envelope(
    input: GeometryAuthoringBatch<'_>,
) -> Result<u64, GeometryAuthoringSemanticError> {
    let primitive_plan =
        CapacityPlan::for_type::<ValidatedGeometryPrimitivePlan>(0, 0, input.primitives().len())
            .map_err(|cause| GeometryAuthoringSemanticError::ScratchCapacity {
                resource: "validated primitive plans",
                cause,
            })?;
    let attribute_plan =
        CapacityPlan::for_type::<ValidatedGeometryAttribute>(0, 0, input.attributes().len())
            .map_err(|cause| GeometryAuthoringSemanticError::ScratchCapacity {
                resource: "validated attribute plans",
                cause,
            })?;
    checked_sum_bytes(
        [
            scratch_envelope(primitive_plan, "validated primitive plans")?,
            scratch_envelope(attribute_plan, "validated attribute plans")?,
        ],
        "geometry shape scratch envelope",
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum GeometryAuthoringLimit {
    Entities,
    Edges,
    References,
    SemanticCells,
    CanonicalBytes,
    Work,
    ScratchBytes,
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum GeometryAuthoringSemanticError {
    #[error("geometry authoring count for {collection} exceeds the u32 canonical domain")]
    CountDomainTooLarge { collection: &'static str },
    #[error("{limit:?} value {actual} exceeds limit {maximum}")]
    LimitExceeded {
        limit: GeometryAuthoringLimit,
        actual: u64,
        maximum: u64,
    },
    #[error("geometry authoring accounting overflowed for {dimension}")]
    AccountingOverflow { dimension: &'static str },
    #[error("logical mesh {logical_mesh} owns an empty primitive range")]
    EmptyLogicalMesh { logical_mesh: usize },
    #[error(
        "logical mesh {logical_mesh} primitive range starts at {actual}; canonical owner coverage expected {expected}"
    )]
    LogicalMeshRangeOrder {
        logical_mesh: usize,
        expected: u32,
        actual: u32,
    },
    #[error(
        "logical mesh {logical_mesh} primitive range exceeds primitive count {primitive_count}"
    )]
    LogicalMeshRangeOutOfBounds {
        logical_mesh: usize,
        primitive_count: usize,
    },
    #[error("flat primitive table has {unowned} rows without exactly one logical mesh owner")]
    UnownedPrimitives { unowned: usize },
    #[error(
        "primitive {primitive} attribute range starts at {actual}; canonical owner coverage expected {expected}"
    )]
    PrimitiveAttributeRangeOrder {
        primitive: usize,
        expected: u32,
        actual: u32,
    },
    #[error("primitive {primitive} attribute range exceeds attribute count {attribute_count}")]
    PrimitiveAttributeRangeOutOfBounds {
        primitive: usize,
        attribute_count: usize,
    },
    #[error("flat attribute table has {unowned} rows without exactly one primitive owner")]
    UnownedAttributes { unowned: usize },
    #[error("primitive {primitive} has no POSITION attribute for {vertex_count} vertices")]
    MissingPosition { primitive: usize, vertex_count: u32 },
    #[error("primitive {primitive} attribute {attribute} repeats an earlier canonical key")]
    DuplicateAttributeKey { primitive: usize, attribute: usize },
    #[error("primitive {primitive} attributes are not in canonical key order at {attribute}")]
    AttributeOrder { primitive: usize, attribute: usize },
    #[error("attribute {attribute} has zero lanes")]
    ZeroAttributeLanes { attribute: usize },
    #[error("attribute {attribute} scalar count is not divisible by lane count {lanes}")]
    AttributeScalarShape { attribute: usize, lanes: u8 },
    #[error("attribute {attribute} uses unsupported built-in domain or scalar format")]
    AttributeSchema { attribute: usize },
    #[error("attribute {attribute} uses a custom semantic before its schema authority exists")]
    CustomAttributeUnsupported { attribute: usize },
    #[error("attribute {attribute} has non-finite f32 source value at scalar {scalar}")]
    NonFiniteAttributeValue { attribute: usize, scalar: usize },
    #[error("attribute {attribute} source projection is valid only for the Corner domain")]
    SourceProjectionDomain { attribute: usize },
    #[error(
        "attribute {attribute} source projection has {actual} indices; expected domain cardinality {expected}"
    )]
    SourceProjectionCardinality {
        attribute: usize,
        expected: u32,
        actual: usize,
    },
    #[error(
        "attribute {attribute} source projection index {index} at element {element} exceeds source count {source_count}"
    )]
    SourceProjectionOutOfRange {
        attribute: usize,
        element: usize,
        index: u32,
        source_count: u32,
    },
    #[error(
        "attribute {attribute} has canonical element count {actual}; {domain:?} requires {expected}"
    )]
    AttributeCardinality {
        attribute: usize,
        domain: AttributeDomain,
        expected: u32,
        actual: u32,
    },
    #[error("sparse attribute {attribute} default has {actual} scalars; expected {lanes}")]
    SparseDefaultShape {
        attribute: usize,
        lanes: u8,
        actual: usize,
    },
    #[error(
        "sparse attribute {attribute} has {value_elements} explicit values for {index_count} indices"
    )]
    SparseValueCount {
        attribute: usize,
        index_count: usize,
        value_elements: usize,
    },
    #[error(
        "sparse attribute {attribute} index {index} at element {element} is outside logical count {logical_count}"
    )]
    SparseIndexOutOfRange {
        attribute: usize,
        element: usize,
        index: u32,
        logical_count: u32,
    },
    #[error(
        "sparse attribute {attribute} indices are not strictly increasing at element {element}"
    )]
    SparseIndexOrder { attribute: usize, element: usize },
    #[error(
        "primitive {primitive} index {index} at corner {corner} exceeds vertex count {vertex_count}"
    )]
    VertexIndexOutOfRange {
        primitive: usize,
        corner: usize,
        index: u32,
        vertex_count: u32,
    },
    #[error(
        "primitive {primitive} {topology:?} corner count {corner_count} is not divisible by {corners_per_face}"
    )]
    ListCornerCount {
        primitive: usize,
        topology: TopologyKind,
        corner_count: u32,
        corners_per_face: u32,
    },
    #[error("primitive {primitive} polygon face {face} has {corner_count} corners")]
    PolygonFaceTooSmall {
        primitive: usize,
        face: usize,
        corner_count: u32,
    },
    #[error("primitive {primitive} polygon corner count overflowed at face {face}")]
    PolygonCornerCountOverflow { primitive: usize, face: usize },
    #[error(
        "primitive {primitive} polygon boundaries contain {boundary_corners} corners; index stream contains {corner_count}"
    )]
    PolygonCornerCountMismatch {
        primitive: usize,
        boundary_corners: u32,
        corner_count: u32,
    },
    #[error(
        "primitive {primitive} material {material} exceeds appearance material count {appearance_material_count}"
    )]
    MaterialOutOfRange {
        primitive: usize,
        material: u32,
        appearance_material_count: u32,
    },
    #[error("mesh instance {instance} owner node {owner} exceeds authored node count {node_count}")]
    InstanceOwnerOutOfRange {
        instance: usize,
        owner: u32,
        node_count: usize,
    },
    #[error(
        "mesh instance {instance} logical mesh {logical_mesh} exceeds logical mesh count {logical_mesh_count}"
    )]
    InstanceMeshOutOfRange {
        instance: usize,
        logical_mesh: u32,
        logical_mesh_count: usize,
    },
    #[error("scratch capacity planning failed for {resource}: {cause}")]
    ScratchCapacity {
        resource: &'static str,
        #[source]
        cause: CapacityPlanError,
    },
    #[error("scratch allocation failed for {resource}")]
    ScratchAllocation { resource: &'static str },
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum GeometryAuthoringFailure<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Semantic(#[from] GeometryAuthoringSemanticError),
    #[error("geometry authoring validation control failed")]
    Control(#[source] E),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum GeometryAttributeScalarKind {
    F32,
    U32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum GeometryAttributePayloadKind {
    Dense,
    Sparse,
}

pub(in crate::canonical_scene::store) struct ValidatedGeometryAttribute {
    input_index: usize,
    scalar_kind: GeometryAttributeScalarKind,
    payload_kind: GeometryAttributePayloadKind,
    source_element_count: u32,
    element_count: u32,
    explicit_element_count: u32,
    canonical_scalar_count: u64,
    payload_block_count: u32,
}

impl ValidatedGeometryAttribute {
    pub(in crate::canonical_scene::store) const fn input_index(&self) -> usize {
        self.input_index
    }

    pub(in crate::canonical_scene::store) const fn scalar_kind(
        &self,
    ) -> GeometryAttributeScalarKind {
        self.scalar_kind
    }

    pub(in crate::canonical_scene::store) const fn payload_kind(
        &self,
    ) -> GeometryAttributePayloadKind {
        self.payload_kind
    }

    pub(in crate::canonical_scene::store) const fn source_element_count(&self) -> u32 {
        self.source_element_count
    }

    pub(in crate::canonical_scene::store) const fn element_count(&self) -> u32 {
        self.element_count
    }

    pub(in crate::canonical_scene::store) const fn explicit_element_count(&self) -> u32 {
        self.explicit_element_count
    }

    pub(in crate::canonical_scene::store) const fn canonical_scalar_count(&self) -> u64 {
        self.canonical_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn payload_block_count(&self) -> u32 {
        self.payload_block_count
    }
}

struct ValidatedGeometryPrimitivePlan {
    input_index: usize,
    topology: TopologyKind,
    vertex_count: u32,
    corner_count: u32,
    face_count: u32,
    index_count: u32,
    attribute_plans: AuthoringRange,
}

pub(in crate::canonical_scene::store) struct ValidatedGeometryPrimitive<'batch, 'input> {
    input: GeometryPrimitiveAuthoring<'input>,
    plan: &'batch ValidatedGeometryPrimitivePlan,
    attributes: &'batch [ValidatedGeometryAttribute],
    input_attributes: &'input [GeometryAttributeAuthoring<'input>],
}

impl<'batch, 'input> ValidatedGeometryPrimitive<'batch, 'input> {
    pub(in crate::canonical_scene::store) const fn input_index(&self) -> usize {
        self.plan.input_index
    }

    pub(in crate::canonical_scene::store) const fn authored_topology(
        &self,
    ) -> GeometryTopologyAuthoring<'input> {
        self.input.topology()
    }

    pub(in crate::canonical_scene::store) const fn authored_indices(
        &self,
    ) -> Presence<&'input [u32]> {
        self.input.indices()
    }

    pub(in crate::canonical_scene::store) const fn attribute_range(&self) -> AuthoringRange {
        self.plan.attribute_plans
    }

    pub(in crate::canonical_scene::store) const fn topology(&self) -> TopologyKind {
        self.plan.topology
    }

    pub(in crate::canonical_scene::store) const fn vertex_count(&self) -> u32 {
        self.plan.vertex_count
    }

    pub(in crate::canonical_scene::store) const fn corner_count(&self) -> u32 {
        self.plan.corner_count
    }

    pub(in crate::canonical_scene::store) const fn face_count(&self) -> u32 {
        self.plan.face_count
    }

    pub(in crate::canonical_scene::store) const fn index_offset(&self) -> Option<u32> {
        match self.input.indices() {
            Presence::Absent => None,
            Presence::Present(_) => Some(0),
        }
    }

    pub(in crate::canonical_scene::store) const fn index_count(&self) -> u32 {
        self.plan.index_count
    }

    pub(in crate::canonical_scene::store) const fn material(&self) -> Option<MaterialLocalId> {
        match self.input.material() {
            Presence::Absent => None,
            Presence::Present(material) => Some(material),
        }
    }

    pub(in crate::canonical_scene::store) fn attributes(
        &self,
    ) -> impl ExactSizeIterator<Item = ValidatedGeometryAttributeView<'_, 'input>> + '_ {
        self.attributes.iter().map(|plan| {
            let input = &self.input_attributes[plan.input_index];
            ValidatedGeometryAttributeView { input, plan }
        })
    }

    pub(in crate::canonical_scene::store) fn attribute(
        &self,
        index: usize,
    ) -> Option<ValidatedGeometryAttributeView<'_, 'input>> {
        let plan = self.attributes.get(index)?;
        let input = self.input_attributes.get(plan.input_index)?;
        Some(ValidatedGeometryAttributeView { input, plan })
    }
}

pub(in crate::canonical_scene::store) struct ValidatedGeometryAttributeView<'view, 'input> {
    input: &'view GeometryAttributeAuthoring<'input>,
    plan: &'view ValidatedGeometryAttribute,
}

impl<'input> ValidatedGeometryAttributeView<'_, 'input> {
    pub(in crate::canonical_scene::store) const fn semantic(&self) -> &AttributeSemantic {
        self.input.semantic()
    }

    pub(in crate::canonical_scene::store) const fn domain(&self) -> AttributeDomain {
        self.input.domain()
    }

    pub(in crate::canonical_scene::store) const fn source_element_count(&self) -> u32 {
        self.plan.source_element_count()
    }

    pub(in crate::canonical_scene::store) const fn element_count(&self) -> u32 {
        self.plan.element_count()
    }

    pub(in crate::canonical_scene::store) const fn scalar_kind(
        &self,
    ) -> GeometryAttributeScalarKind {
        self.plan.scalar_kind()
    }

    pub(in crate::canonical_scene::store) const fn payload_kind(
        &self,
    ) -> GeometryAttributePayloadKind {
        self.plan.payload_kind()
    }

    pub(in crate::canonical_scene::store) const fn explicit_element_count(&self) -> u32 {
        self.plan.explicit_element_count()
    }

    pub(in crate::canonical_scene::store) const fn canonical_scalar_count(&self) -> u64 {
        self.plan.canonical_scalar_count()
    }

    pub(in crate::canonical_scene::store) const fn payload_block_count(&self) -> u32 {
        self.plan.payload_block_count()
    }

    pub(in crate::canonical_scene::store) const fn authored_payload(
        &self,
    ) -> GeometryAttributePayloadAuthoring<'input> {
        self.input.payload()
    }

    pub(in crate::canonical_scene::store) const fn lanes(&self) -> u8 {
        match self.input.payload() {
            GeometryAttributePayloadAuthoring::DenseF32 { lanes, .. }
            | GeometryAttributePayloadAuthoring::DenseU32 { lanes, .. }
            | GeometryAttributePayloadAuthoring::SparseF32 { lanes, .. }
            | GeometryAttributePayloadAuthoring::SparseU32 { lanes, .. } => lanes,
        }
    }
}

#[must_use = "dropping the validated geometry batch releases its compiled-plan reservation"]
pub(in crate::canonical_scene::store) struct ValidatedGeometryBatch<'input, R> {
    input: GeometryAuthoringBatch<'input>,
    logical_mesh_count: u32,
    primitive_count: u32,
    mesh_instance_count: u32,
    attribute_count: u32,
    f32_scalar_count: u64,
    u32_scalar_count: u64,
    payload_block_count: u32,
    entity_count: u64,
    edge_count: u64,
    reference_count: u64,
    semantic_cell_count: u64,
    canonical_bytes: u64,
    validation_work: u64,
    scratch_bytes: u64,
    primitive_plans: Vec<ValidatedGeometryPrimitivePlan>,
    attribute_plans: Vec<ValidatedGeometryAttribute>,
    #[cfg(test)]
    storage_drop_probe: Option<GeometryStorageDropProbe>,
    // Every compiled-plan allocation above must drop before its accounting token.
    reservation: R,
}

impl<'input, R> ValidatedGeometryBatch<'input, R> {
    pub(in crate::canonical_scene::store) const fn input(&self) -> GeometryAuthoringBatch<'input> {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn logical_mesh_count(&self) -> u32 {
        self.logical_mesh_count
    }

    pub(in crate::canonical_scene::store) const fn primitive_count(&self) -> u32 {
        self.primitive_count
    }

    pub(in crate::canonical_scene::store) const fn mesh_instance_count(&self) -> u32 {
        self.mesh_instance_count
    }

    pub(in crate::canonical_scene::store) const fn attribute_count(&self) -> u32 {
        self.attribute_count
    }

    pub(in crate::canonical_scene::store) const fn f32_scalar_count(&self) -> u64 {
        self.f32_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn u32_scalar_count(&self) -> u64 {
        self.u32_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn payload_block_count(&self) -> u32 {
        self.payload_block_count
    }

    pub(in crate::canonical_scene::store) const fn entity_count(&self) -> u64 {
        self.entity_count
    }

    pub(in crate::canonical_scene::store) const fn edge_count(&self) -> u64 {
        self.edge_count
    }

    pub(in crate::canonical_scene::store) const fn reference_count(&self) -> u64 {
        self.reference_count
    }

    pub(in crate::canonical_scene::store) const fn semantic_cell_count(&self) -> u64 {
        self.semantic_cell_count
    }

    pub(in crate::canonical_scene::store) const fn canonical_bytes(&self) -> u64 {
        self.canonical_bytes
    }

    pub(in crate::canonical_scene::store) const fn validation_work(&self) -> u64 {
        self.validation_work
    }

    pub(in crate::canonical_scene::store) const fn scratch_bytes(&self) -> u64 {
        self.scratch_bytes
    }

    pub(in crate::canonical_scene::store) fn primitive(
        &self,
        index: usize,
    ) -> Option<ValidatedGeometryPrimitive<'_, 'input>> {
        let plan = self.primitive_plans.get(index)?;
        let input = *self.input.primitives().get(plan.input_index)?;
        let range = checked_range(plan.attribute_plans, self.attribute_plans.len()).ok()?;
        Some(ValidatedGeometryPrimitive {
            input,
            plan,
            attributes: self.attribute_plans.get(range.clone())?,
            input_attributes: self.input.attributes(),
        })
    }

    pub(in crate::canonical_scene::store) fn attribute(
        &self,
        index: usize,
    ) -> Option<ValidatedGeometryAttributeView<'_, 'input>> {
        let plan = self.attribute_plans.get(index)?;
        let input = self.input.attributes().get(plan.input_index)?;
        Some(ValidatedGeometryAttributeView { input, plan })
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_storage_drop_probe(
        &mut self,
        stage: std::rc::Rc<std::cell::Cell<u8>>,
    ) {
        self.storage_drop_probe = Some(GeometryStorageDropProbe(stage));
    }
}

impl<R> fmt::Debug for ValidatedGeometryBatch<'_, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedGeometryBatch")
            .field("logical_mesh_count", &self.logical_mesh_count)
            .field("primitive_count", &self.primitive_count)
            .field("mesh_instance_count", &self.mesh_instance_count)
            .field("attribute_count", &self.attribute_count)
            .field("f32_scalar_count", &self.f32_scalar_count)
            .field("u32_scalar_count", &self.u32_scalar_count)
            .field("payload_block_count", &self.payload_block_count)
            .field("entity_count", &self.entity_count)
            .field("edge_count", &self.edge_count)
            .field("reference_count", &self.reference_count)
            .field("semantic_cell_count", &self.semantic_cell_count)
            .field("canonical_bytes", &self.canonical_bytes)
            .field("validation_work", &self.validation_work)
            .field("scratch_bytes", &self.scratch_bytes)
            .field("reservation", &"<held>")
            .finish()
    }
}

#[cfg(test)]
struct GeometryStorageDropProbe(std::rc::Rc<std::cell::Cell<u8>>);

#[cfg(test)]
impl Drop for GeometryStorageDropProbe {
    fn drop(&mut self) {
        assert_eq!(
            self.0.replace(1),
            0,
            "compiled geometry storage must begin the coupled drop sequence"
        );
    }
}

struct GeometryValidationStorage<R> {
    primitive_plans: Vec<ValidatedGeometryPrimitivePlan>,
    attribute_plans: Vec<ValidatedGeometryAttribute>,
    // Every allocation above must drop before its accounting token.
    reservation: R,
}

type GeometryValidationResult<T, C> =
    Result<T, GeometryAuthoringFailure<<C as SceneValidationControl>::Error>>;

struct GeometryValidationContext<'control, C: SceneValidationControl> {
    control: &'control mut C,
    limits: SceneValidationLimits,
    work: u64,
    work_since_poll: u64,
    edges: u64,
    references: u64,
    semantic_cells: u64,
    f32_scalars: u64,
    u32_scalars: u64,
    payload_blocks: u32,
    canonical_bytes: u64,
}

impl<'control, C: SceneValidationControl> GeometryValidationContext<'control, C> {
    const fn new(limits: SceneValidationLimits, control: &'control mut C) -> Self {
        Self {
            control,
            limits,
            work: 0,
            work_since_poll: 0,
            edges: 0,
            references: 0,
            semantic_cells: 0,
            f32_scalars: 0,
            u32_scalars: 0,
            payload_blocks: 0,
            canonical_bytes: 0,
        }
    }

    fn poll_now(&mut self) -> GeometryValidationResult<(), C> {
        self.control
            .poll()
            .map_err(GeometryAuthoringFailure::Control)?;
        self.work_since_poll = 0;
        Ok(())
    }

    fn finish(&mut self) -> GeometryValidationResult<(), C> {
        self.poll_now()
    }

    fn work(&mut self, kind: SceneValidationWork, amount: u64) -> GeometryValidationResult<(), C> {
        let final_work = self.work.checked_add(amount).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "validation work",
            },
        )?;
        enforce_limit::<C::Error>(
            GeometryAuthoringLimit::Work,
            final_work,
            self.limits.max_work(),
        )?;

        let mut remaining = amount;
        while remaining > 0 {
            let until_poll = SCENE_VALIDATION_POLL_INTERVAL - self.work_since_poll;
            let chunk = remaining.min(until_poll);
            self.control
                .charge_work(kind, chunk)
                .map_err(GeometryAuthoringFailure::Control)?;
            self.work = self.work.checked_add(chunk).ok_or(
                GeometryAuthoringSemanticError::AccountingOverflow {
                    dimension: "validation work",
                },
            )?;
            self.work_since_poll += chunk;
            remaining -= chunk;
            if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
                self.poll_now()?;
            }
        }
        Ok(())
    }

    fn add_relations(&mut self, amount: u64) -> GeometryValidationResult<(), C> {
        let final_edges = self
            .edges
            .checked_add(amount)
            .ok_or(GeometryAuthoringSemanticError::AccountingOverflow { dimension: "edges" })?;
        let final_references = self.references.checked_add(amount).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "references",
            },
        )?;
        enforce_limit::<C::Error>(
            GeometryAuthoringLimit::Edges,
            final_edges,
            self.limits.max_edges(),
        )?;
        enforce_limit::<C::Error>(
            GeometryAuthoringLimit::References,
            final_references,
            self.limits.max_references(),
        )?;

        self.edges = final_edges;
        self.references = final_references;
        self.work(SceneValidationWork::GraphEdge, amount)?;
        self.work(SceneValidationWork::Reference, amount)
    }

    fn add_payload(
        &mut self,
        f32_scalars: u64,
        u32_scalars: u64,
        payload_blocks: u32,
    ) -> GeometryValidationResult<(), C> {
        let added_cells = f32_scalars.checked_add(u32_scalars).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "semantic cells",
            },
        )?;
        let final_cells = self.semantic_cells.checked_add(added_cells).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "semantic cells",
            },
        )?;
        let final_f32 = self.f32_scalars.checked_add(f32_scalars).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "f32 scalar count",
            },
        )?;
        let final_u32 = self.u32_scalars.checked_add(u32_scalars).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "u32 scalar count",
            },
        )?;
        let final_blocks = self.payload_blocks.checked_add(payload_blocks).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "payload block count",
            },
        )?;
        let added_bytes = checked_sum_bytes(
            [
                checked_bytes(
                    f32_scalars,
                    LOGICAL_F32_BYTES,
                    "canonical f32 scalar payload",
                )?,
                checked_bytes(
                    u32_scalars,
                    LOGICAL_U32_BYTES,
                    "canonical u32 scalar payload",
                )?,
            ],
            "canonical scalar payload",
        )?;
        let final_canonical_bytes = self.canonical_bytes.checked_add(added_bytes).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "canonical geometry logical footprint",
            },
        )?;
        enforce_limit::<C::Error>(
            GeometryAuthoringLimit::SemanticCells,
            final_cells,
            self.limits.max_payload_cells(),
        )?;
        enforce_limit::<C::Error>(
            GeometryAuthoringLimit::CanonicalBytes,
            final_canonical_bytes,
            self.limits.max_canonical_bytes(),
        )?;

        self.semantic_cells = final_cells;
        self.f32_scalars = final_f32;
        self.u32_scalars = final_u32;
        self.payload_blocks = final_blocks;
        self.canonical_bytes = final_canonical_bytes;
        Ok(())
    }

    fn set_structural_bytes(&mut self, bytes: u64) -> GeometryValidationResult<(), C> {
        enforce_limit::<C::Error>(
            GeometryAuthoringLimit::CanonicalBytes,
            bytes,
            self.limits.max_canonical_bytes(),
        )?;
        self.canonical_bytes = bytes;
        Ok(())
    }

    fn scratch_plan<T>(
        &self,
        count: usize,
        resource: &'static str,
    ) -> Result<CapacityPlan, GeometryAuthoringSemanticError> {
        CapacityPlan::for_type::<T>(0, 0, count)
            .map_err(|cause| GeometryAuthoringSemanticError::ScratchCapacity { resource, cause })
    }

    fn reserve_scratch(&mut self, bytes: u64) -> GeometryValidationResult<C::Reservation, C> {
        enforce_limit::<C::Error>(
            GeometryAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        let reservation = self
            .control
            .reserve(SceneReservationKind::ValidationScratch, bytes)
            .map_err(GeometryAuthoringFailure::Control)?;
        self.control
            .validate_reservation(&reservation, SceneReservationKind::ValidationScratch)
            .map_err(GeometryAuthoringFailure::Control)?;
        Ok(reservation)
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut C::Reservation,
        bytes: u64,
    ) -> GeometryValidationResult<(), C> {
        enforce_limit::<C::Error>(
            GeometryAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        self.control
            .resize_reservation(reservation, SceneReservationKind::ValidationScratch, bytes)
            .map_err(GeometryAuthoringFailure::Control)
    }

    fn allocate_scratch<T>(
        &self,
        plan: CapacityPlan,
        resource: &'static str,
    ) -> Result<(Vec<T>, u64), GeometryAuthoringSemanticError> {
        plan.verify_preflight(0, 0)
            .map_err(|cause| GeometryAuthoringSemanticError::ScratchCapacity { resource, cause })?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| GeometryAuthoringSemanticError::ScratchAllocation { resource })?;
        let actual_bytes = plan
            .finish(values.len(), values.capacity())
            .map_err(|cause| GeometryAuthoringSemanticError::ScratchCapacity { resource, cause })?;
        Ok((values, actual_bytes))
    }
}

pub(in crate::canonical_scene::store) fn validate_geometry_batch<'input, O, PP, U>(
    input: GeometryAuthoringBatch<'input>,
    authored_node_count: usize,
    appearance_material_count: u32,
    limits: SceneValidationLimits,
    control: &mut OperationAuthoringControl<'_, O, PP, U>,
) -> Result<
    ValidatedGeometryBatch<'input, RetainedStoreReservation<O>>,
    GeometryAuthoringFailure<OperationAuthoringControlError>,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    validate_geometry_batch_inner(
        input,
        authored_node_count,
        appearance_material_count,
        limits,
        control,
    )
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn validate_geometry_batch_with_control<'input, C>(
    input: GeometryAuthoringBatch<'input>,
    authored_node_count: usize,
    appearance_material_count: u32,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedGeometryBatch<'input, C::Reservation>, GeometryAuthoringFailure<C::Error>>
where
    C: SceneValidationControl,
{
    validate_geometry_batch_inner(
        input,
        authored_node_count,
        appearance_material_count,
        limits,
        control,
    )
}

fn validate_geometry_batch_inner<'input, C>(
    input: GeometryAuthoringBatch<'input>,
    authored_node_count: usize,
    appearance_material_count: u32,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedGeometryBatch<'input, C::Reservation>, GeometryAuthoringFailure<C::Error>>
where
    C: SceneValidationControl,
{
    let mut context = GeometryValidationContext::new(limits, control);
    context.poll_now()?;

    let logical_mesh_count = collection_count(input.logical_meshes().len(), "logical meshes")?;
    let primitive_count = collection_count(input.primitives().len(), "primitives")?;
    let attribute_count = collection_count(input.attributes().len(), "attributes")?;
    let mesh_instance_count = collection_count(input.mesh_instances().len(), "mesh instances")?;
    let entity_count = checked_sum(
        [
            u64::from(logical_mesh_count),
            u64::from(primitive_count),
            u64::from(attribute_count),
            u64::from(mesh_instance_count),
        ],
        "geometry entities",
    )?;
    enforce_limit::<C::Error>(
        GeometryAuthoringLimit::Entities,
        entity_count,
        limits.max_entities(),
    )?;
    let structural_bytes = geometry_structural_bytes(input, &mut context)?;
    context.set_structural_bytes(structural_bytes)?;

    let primitive_plan = context.scratch_plan::<ValidatedGeometryPrimitivePlan>(
        input.primitives().len(),
        "validated primitive plans",
    )?;
    let attribute_plan = context.scratch_plan::<ValidatedGeometryAttribute>(
        input.attributes().len(),
        "validated attribute plans",
    )?;
    let scratch_envelope = checked_sum_bytes(
        [
            scratch_envelope(primitive_plan, "validated primitive plans")?,
            scratch_envelope(attribute_plan, "validated attribute plans")?,
        ],
        "geometry scratch envelope",
    )?;

    let mut reservation = context.reserve_scratch(scratch_envelope)?;
    let (primitive_plans, primitive_plan_bytes) =
        context.allocate_scratch(primitive_plan, "validated primitive plans")?;
    let (attribute_plans, attribute_plan_bytes) =
        context.allocate_scratch(attribute_plan, "validated attribute plans")?;
    let scratch_actual = checked_sum_bytes(
        [primitive_plan_bytes, attribute_plan_bytes],
        "geometry scratch allocation",
    )?;
    context.resize_scratch(&mut reservation, scratch_actual)?;
    let mut storage = GeometryValidationStorage {
        primitive_plans,
        attribute_plans,
        reservation,
    };

    let mut expected_primitive_offset = 0_u32;
    for (logical_mesh_index, logical_mesh) in input.logical_meshes().iter().copied().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        let range = logical_mesh.primitives();
        if range.count() == 0 {
            return Err(GeometryAuthoringSemanticError::EmptyLogicalMesh {
                logical_mesh: logical_mesh_index,
            }
            .into());
        }
        if range.offset() != expected_primitive_offset {
            return Err(GeometryAuthoringSemanticError::LogicalMeshRangeOrder {
                logical_mesh: logical_mesh_index,
                expected: expected_primitive_offset,
                actual: range.offset(),
            }
            .into());
        }
        let Some(end) = range.offset().checked_add(range.count()) else {
            return Err(
                GeometryAuthoringSemanticError::LogicalMeshRangeOutOfBounds {
                    logical_mesh: logical_mesh_index,
                    primitive_count: input.primitives().len(),
                }
                .into(),
            );
        };
        if end > primitive_count {
            return Err(
                GeometryAuthoringSemanticError::LogicalMeshRangeOutOfBounds {
                    logical_mesh: logical_mesh_index,
                    primitive_count: input.primitives().len(),
                }
                .into(),
            );
        }
        context.add_relations(u64::from(range.count()))?;
        expected_primitive_offset = end;
    }
    if expected_primitive_offset != primitive_count {
        let unowned = primitive_count
            .checked_sub(expected_primitive_offset)
            .ok_or(GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "unowned primitive count",
            })?;
        return Err(GeometryAuthoringSemanticError::UnownedPrimitives {
            unowned: u32_to_usize(unowned, "unowned primitive count")?,
        }
        .into());
    }

    let mut expected_attribute_offset = 0_u32;
    for (primitive_index, primitive) in input.primitives().iter().copied().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        let attribute_range = primitive.attributes();
        if attribute_range.offset() != expected_attribute_offset {
            return Err(
                GeometryAuthoringSemanticError::PrimitiveAttributeRangeOrder {
                    primitive: primitive_index,
                    expected: expected_attribute_offset,
                    actual: attribute_range.offset(),
                }
                .into(),
            );
        }
        let Some(attribute_end) = attribute_range
            .offset()
            .checked_add(attribute_range.count())
        else {
            return Err(
                GeometryAuthoringSemanticError::PrimitiveAttributeRangeOutOfBounds {
                    primitive: primitive_index,
                    attribute_count: input.attributes().len(),
                }
                .into(),
            );
        };
        if attribute_end > attribute_count {
            return Err(
                GeometryAuthoringSemanticError::PrimitiveAttributeRangeOutOfBounds {
                    primitive: primitive_index,
                    attribute_count: input.attributes().len(),
                }
                .into(),
            );
        }
        context.add_relations(u64::from(attribute_range.count()))?;

        if let Presence::Present(material) = primitive.material() {
            if material.index() >= appearance_material_count {
                return Err(GeometryAuthoringSemanticError::MaterialOutOfRange {
                    primitive: primitive_index,
                    material: material.index(),
                    appearance_material_count,
                }
                .into());
            }
            context.add_relations(1)?;
        }

        let attribute_indices = checked_range(attribute_range, input.attributes().len())?;
        let plan = validate_primitive(
            primitive_index,
            primitive,
            attribute_indices,
            input.attributes(),
            &mut storage.attribute_plans,
            &mut context,
        )?;
        storage.primitive_plans.push(plan);
        expected_attribute_offset = attribute_end;
    }
    if expected_attribute_offset != attribute_count {
        let unowned = attribute_count
            .checked_sub(expected_attribute_offset)
            .ok_or(GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "unowned attribute count",
            })?;
        return Err(GeometryAuthoringSemanticError::UnownedAttributes {
            unowned: u32_to_usize(unowned, "unowned attribute count")?,
        }
        .into());
    }

    for (instance_index, instance) in input.mesh_instances().iter().copied().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        let owner = instance.owner().index();
        if u32_to_usize(owner, "mesh instance owner")? >= authored_node_count {
            return Err(GeometryAuthoringSemanticError::InstanceOwnerOutOfRange {
                instance: instance_index,
                owner,
                node_count: authored_node_count,
            }
            .into());
        }
        context.add_relations(1)?;

        let logical_mesh = instance.logical_mesh().index();
        if logical_mesh >= logical_mesh_count {
            return Err(GeometryAuthoringSemanticError::InstanceMeshOutOfRange {
                instance: instance_index,
                logical_mesh,
                logical_mesh_count: input.logical_meshes().len(),
            }
            .into());
        }
        context.add_relations(1)?;
    }

    let canonical_bytes = context.canonical_bytes;
    let validation_work = context.work;
    let GeometryValidationStorage {
        primitive_plans,
        attribute_plans,
        reservation,
    } = storage;
    let validated = ValidatedGeometryBatch {
        input,
        logical_mesh_count,
        primitive_count,
        mesh_instance_count,
        attribute_count,
        f32_scalar_count: context.f32_scalars,
        u32_scalar_count: context.u32_scalars,
        payload_block_count: context.payload_blocks,
        entity_count,
        edge_count: context.edges,
        reference_count: context.references,
        semantic_cell_count: context.semantic_cells,
        canonical_bytes,
        validation_work,
        scratch_bytes: scratch_actual,
        primitive_plans,
        attribute_plans,
        #[cfg(test)]
        storage_drop_probe: None,
        reservation,
    };
    context.finish()?;
    Ok(validated)
}

fn validate_primitive<C>(
    primitive_index: usize,
    primitive: GeometryPrimitiveAuthoring<'_>,
    attribute_indices: Range<usize>,
    attributes: &[GeometryAttributeAuthoring<'_>],
    attribute_plans: &mut Vec<ValidatedGeometryAttribute>,
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<ValidatedGeometryPrimitivePlan, C>
where
    C: SceneValidationControl,
{
    let mut previous_key: Option<(&AttributeSemantic, AttributeDomain)> = None;
    let mut position_index = None;
    let mut inferred_vertex_count = 0_u32;
    for attribute_index in attribute_indices.clone() {
        context.work(SceneValidationWork::Entity, 1)?;
        let attribute = attributes.get(attribute_index).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "primitive attribute lookup",
            },
        )?;
        let key = (attribute.semantic(), attribute.domain());
        if let Some(previous) = previous_key {
            match previous.cmp(&key) {
                std::cmp::Ordering::Equal => {
                    return Err(GeometryAuthoringSemanticError::DuplicateAttributeKey {
                        primitive: primitive_index,
                        attribute: attribute_index,
                    }
                    .into());
                }
                std::cmp::Ordering::Greater => {
                    return Err(GeometryAuthoringSemanticError::AttributeOrder {
                        primitive: primitive_index,
                        attribute: attribute_index,
                    }
                    .into());
                }
                std::cmp::Ordering::Less => {}
            }
        }
        if attribute.semantic().kind() == AttributeSemanticKind::Position {
            position_index = Some(attribute_index);
        }
        if attribute.domain() == AttributeDomain::Vertex {
            inferred_vertex_count =
                inferred_vertex_count.max(attribute_element_count(attribute_index, attribute)?);
        }
        previous_key = Some(key);
    }

    let mut position_plan = match position_index {
        Some(attribute_index) => Some((
            attribute_index,
            validate_attribute(attribute_index, &attributes[attribute_index], None, context)?,
        )),
        None => None,
    };
    let vertex_count = if let Some((_, plan)) = &position_plan {
        plan.element_count()
    } else {
        infer_missing_position_vertex_count(primitive, inferred_vertex_count, context)?
    };
    if vertex_count > 0 && position_plan.is_none() {
        return Err(GeometryAuthoringSemanticError::MissingPosition {
            primitive: primitive_index,
            vertex_count,
        }
        .into());
    }

    let (corner_count, index_count) =
        validate_index_stream(primitive_index, primitive.indices(), vertex_count, context)?;
    let (topology, face_count) =
        validate_topology(primitive_index, primitive.topology(), corner_count, context)?;

    for attribute_index in attribute_indices {
        context.work(SceneValidationWork::Traversal, 1)?;
        let attribute = &attributes[attribute_index];
        if attribute.semantic().kind() == AttributeSemanticKind::Custom {
            return Err(GeometryAuthoringSemanticError::CustomAttributeUnsupported {
                attribute: attribute_index,
            }
            .into());
        }
        let expected = domain_cardinality(
            attribute_index,
            attribute.domain(),
            vertex_count,
            corner_count,
            face_count,
        )?;
        let plan = match position_plan.take() {
            Some((position, plan)) if position == attribute_index => plan,
            Some((position, plan)) => {
                position_plan = Some((position, plan));
                validate_attribute(attribute_index, attribute, Some(expected), context)?
            }
            None => validate_attribute(attribute_index, attribute, Some(expected), context)?,
        };
        if plan.element_count() != expected {
            return Err(GeometryAuthoringSemanticError::AttributeCardinality {
                attribute: attribute_index,
                domain: attribute.domain(),
                expected,
                actual: plan.element_count(),
            }
            .into());
        }
        attribute_plans.push(plan);
    }

    Ok(ValidatedGeometryPrimitivePlan {
        input_index: primitive_index,
        topology,
        vertex_count,
        corner_count,
        face_count,
        index_count,
        attribute_plans: primitive.attributes(),
    })
}

fn infer_missing_position_vertex_count<C>(
    primitive: GeometryPrimitiveAuthoring<'_>,
    mut inferred: u32,
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<u32, C>
where
    C: SceneValidationControl,
{
    if let Presence::Present(indices) = primitive.indices() {
        for &index in indices {
            context.work(SceneValidationWork::PayloadCell, 1)?;
            let declared_count = index.checked_add(1).ok_or(
                GeometryAuthoringSemanticError::CountDomainTooLarge {
                    collection: "primitive vertices",
                },
            )?;
            inferred = inferred.max(declared_count);
        }
    }
    Ok(inferred)
}

fn domain_cardinality(
    attribute_index: usize,
    domain: AttributeDomain,
    vertex_count: u32,
    corner_count: u32,
    face_count: u32,
) -> Result<u32, GeometryAuthoringSemanticError> {
    match domain {
        AttributeDomain::Vertex => Ok(vertex_count),
        AttributeDomain::Corner => Ok(corner_count),
        AttributeDomain::Face => Ok(face_count),
        AttributeDomain::Instance => Err(GeometryAuthoringSemanticError::AttributeSchema {
            attribute: attribute_index,
        }),
    }
}

fn validate_index_stream<C>(
    primitive_index: usize,
    indices: Presence<&[u32]>,
    vertex_count: u32,
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<(u32, u32), C>
where
    C: SceneValidationControl,
{
    match indices {
        Presence::Absent => Ok((vertex_count, 0)),
        Presence::Present(indices) => {
            let index_count = collection_count(indices.len(), "primitive indices")?;
            context.add_payload(0, u64::from(index_count), block_for_len(indices.len()))?;
            for (corner, &index) in indices.iter().enumerate() {
                context.work(SceneValidationWork::PayloadCell, 1)?;
                if index >= vertex_count {
                    return Err(GeometryAuthoringSemanticError::VertexIndexOutOfRange {
                        primitive: primitive_index,
                        corner,
                        index,
                        vertex_count,
                    }
                    .into());
                }
            }
            Ok((index_count, index_count))
        }
    }
}

fn validate_topology<C>(
    primitive_index: usize,
    topology: GeometryTopologyAuthoring<'_>,
    corner_count: u32,
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<(TopologyKind, u32), C>
where
    C: SceneValidationControl,
{
    match topology {
        GeometryTopologyAuthoring::Points => Ok((TopologyKind::Points, corner_count)),
        GeometryTopologyAuthoring::LineList => Ok((
            TopologyKind::LineList,
            list_face_count(primitive_index, TopologyKind::LineList, corner_count, 2)?,
        )),
        GeometryTopologyAuthoring::LineStrip => {
            Ok((TopologyKind::LineStrip, corner_count.saturating_sub(1)))
        }
        GeometryTopologyAuthoring::LineLoop => Ok((
            TopologyKind::LineLoop,
            if corner_count < 2 { 0 } else { corner_count },
        )),
        GeometryTopologyAuthoring::TriangleList => Ok((
            TopologyKind::TriangleList,
            list_face_count(primitive_index, TopologyKind::TriangleList, corner_count, 3)?,
        )),
        GeometryTopologyAuthoring::TriangleStrip => {
            Ok((TopologyKind::TriangleStrip, corner_count.saturating_sub(2)))
        }
        GeometryTopologyAuthoring::TriangleFan => {
            Ok((TopologyKind::TriangleFan, corner_count.saturating_sub(2)))
        }
        GeometryTopologyAuthoring::Polygons(boundaries) => {
            let face_count = collection_count(boundaries.len(), "polygon faces")?;
            context.add_payload(0, u64::from(face_count), block_for_len(boundaries.len()))?;
            let mut boundary_corners = 0_u32;
            for (face, &face_corner_count) in boundaries.iter().enumerate() {
                context.work(SceneValidationWork::PayloadCell, 1)?;
                if face_corner_count < 3 {
                    return Err(GeometryAuthoringSemanticError::PolygonFaceTooSmall {
                        primitive: primitive_index,
                        face,
                        corner_count: face_corner_count,
                    }
                    .into());
                }
                boundary_corners = boundary_corners.checked_add(face_corner_count).ok_or(
                    GeometryAuthoringSemanticError::PolygonCornerCountOverflow {
                        primitive: primitive_index,
                        face,
                    },
                )?;
            }
            if boundary_corners != corner_count {
                return Err(GeometryAuthoringSemanticError::PolygonCornerCountMismatch {
                    primitive: primitive_index,
                    boundary_corners,
                    corner_count,
                }
                .into());
            }
            Ok((TopologyKind::Polygons, face_count))
        }
    }
}

fn list_face_count(
    primitive: usize,
    topology: TopologyKind,
    corner_count: u32,
    corners_per_face: u32,
) -> Result<u32, GeometryAuthoringSemanticError> {
    if corner_count.is_multiple_of(corners_per_face) {
        Ok(corner_count / corners_per_face)
    } else {
        Err(GeometryAuthoringSemanticError::ListCornerCount {
            primitive,
            topology,
            corner_count,
            corners_per_face,
        })
    }
}

fn validate_attribute<C>(
    attribute_index: usize,
    attribute: &GeometryAttributeAuthoring<'_>,
    expected_cardinality: Option<u32>,
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<ValidatedGeometryAttribute, C>
where
    C: SceneValidationControl,
{
    match attribute.payload() {
        GeometryAttributePayloadAuthoring::DenseF32 {
            lanes,
            source_values,
            source_indices,
        } => {
            let source_element_count = scalar_element_count(
                attribute_index,
                lanes,
                source_values.len(),
                "f32 attribute source elements",
            )?;
            validate_attribute_schema(
                attribute_index,
                attribute,
                GeometryAttributeScalarKind::F32,
                lanes,
            )?;
            let (element_count, projection) = match source_indices {
                Presence::Absent => (source_element_count, None),
                Presence::Present(indices) => {
                    if attribute.domain() != AttributeDomain::Corner {
                        return Err(GeometryAuthoringSemanticError::SourceProjectionDomain {
                            attribute: attribute_index,
                        }
                        .into());
                    }
                    let projected_count =
                        collection_count(indices.len(), "attribute source projection elements")?;
                    let expected = expected_cardinality.unwrap_or(projected_count);
                    if projected_count != expected {
                        return Err(
                            GeometryAuthoringSemanticError::SourceProjectionCardinality {
                                attribute: attribute_index,
                                expected,
                                actual: indices.len(),
                            }
                            .into(),
                        );
                    }
                    (projected_count, Some(indices))
                }
            };
            check_attribute_cardinality(
                attribute_index,
                attribute.domain(),
                element_count,
                expected_cardinality,
            )?;
            let canonical_scalar_count = checked_scalar_count(element_count, lanes)?;
            let payload_block_count = block_for_u64(canonical_scalar_count);
            context.add_payload(canonical_scalar_count, 0, payload_block_count)?;

            inspect_f32_values(attribute_index, source_values, context)?;
            if let Some(indices) = projection {
                for (element, &index) in indices.iter().enumerate() {
                    context.work(SceneValidationWork::PayloadCell, 1)?;
                    if index >= source_element_count {
                        return Err(GeometryAuthoringSemanticError::SourceProjectionOutOfRange {
                            attribute: attribute_index,
                            element,
                            index,
                            source_count: source_element_count,
                        }
                        .into());
                    }
                }
            }
            Ok(ValidatedGeometryAttribute {
                input_index: attribute_index,
                scalar_kind: GeometryAttributeScalarKind::F32,
                payload_kind: GeometryAttributePayloadKind::Dense,
                source_element_count,
                element_count,
                explicit_element_count: element_count,
                canonical_scalar_count,
                payload_block_count,
            })
        }
        GeometryAttributePayloadAuthoring::DenseU32 { lanes, values } => {
            let element_count = scalar_element_count(
                attribute_index,
                lanes,
                values.len(),
                "u32 attribute elements",
            )?;
            validate_attribute_schema(
                attribute_index,
                attribute,
                GeometryAttributeScalarKind::U32,
                lanes,
            )?;
            check_attribute_cardinality(
                attribute_index,
                attribute.domain(),
                element_count,
                expected_cardinality,
            )?;
            let canonical_scalar_count = usize_to_u64(values.len(), "u32 attribute scalars")?;
            let payload_block_count = block_for_len(values.len());
            context.add_payload(0, canonical_scalar_count, payload_block_count)?;
            inspect_u32_values(values, context)?;
            Ok(ValidatedGeometryAttribute {
                input_index: attribute_index,
                scalar_kind: GeometryAttributeScalarKind::U32,
                payload_kind: GeometryAttributePayloadKind::Dense,
                source_element_count: element_count,
                element_count,
                explicit_element_count: element_count,
                canonical_scalar_count,
                payload_block_count,
            })
        }
        GeometryAttributePayloadAuthoring::SparseF32 {
            lanes,
            logical_count,
            indices,
            values,
            default,
        } => validate_sparse_attribute(
            attribute_index,
            attribute,
            lanes,
            logical_count,
            indices,
            values,
            default,
            expected_cardinality,
            context,
        ),
        GeometryAttributePayloadAuthoring::SparseU32 {
            lanes,
            logical_count,
            indices,
            values,
            default,
        } => validate_sparse_attribute(
            attribute_index,
            attribute,
            lanes,
            logical_count,
            indices,
            values,
            default,
            expected_cardinality,
            context,
        ),
    }
}

trait GeometryScalar: Copy {
    const KIND: GeometryAttributeScalarKind;

    fn inspect<C: SceneValidationControl>(
        attribute_index: usize,
        values: &[Self],
        scalar_offset: usize,
        context: &mut GeometryValidationContext<'_, C>,
    ) -> GeometryValidationResult<(), C>;
}

impl GeometryScalar for f32 {
    const KIND: GeometryAttributeScalarKind = GeometryAttributeScalarKind::F32;

    fn inspect<C: SceneValidationControl>(
        attribute_index: usize,
        values: &[Self],
        scalar_offset: usize,
        context: &mut GeometryValidationContext<'_, C>,
    ) -> GeometryValidationResult<(), C> {
        inspect_f32_values_from(attribute_index, values, scalar_offset, context)
    }
}

impl GeometryScalar for u32 {
    const KIND: GeometryAttributeScalarKind = GeometryAttributeScalarKind::U32;

    fn inspect<C: SceneValidationControl>(
        _attribute_index: usize,
        values: &[Self],
        _scalar_offset: usize,
        context: &mut GeometryValidationContext<'_, C>,
    ) -> GeometryValidationResult<(), C> {
        inspect_u32_values(values, context)
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_sparse_attribute<C, T>(
    attribute_index: usize,
    attribute: &GeometryAttributeAuthoring<'_>,
    lanes: u8,
    logical_count: u32,
    indices: &[u32],
    values: &[T],
    default: &[T],
    expected_cardinality: Option<u32>,
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<ValidatedGeometryAttribute, C>
where
    C: SceneValidationControl,
    T: GeometryScalar,
{
    let scalar_kind = T::KIND;
    let explicit_element_count = scalar_element_count(
        attribute_index,
        lanes,
        values.len(),
        "sparse attribute explicit elements",
    )?;
    validate_attribute_schema(attribute_index, attribute, scalar_kind, lanes)?;
    if default.len() != usize::from(lanes) {
        return Err(GeometryAuthoringSemanticError::SparseDefaultShape {
            attribute: attribute_index,
            lanes,
            actual: default.len(),
        }
        .into());
    }
    let explicit_element_count_usize =
        u32_to_usize(explicit_element_count, "sparse explicit element count")?;
    let index_count = collection_count(indices.len(), "sparse attribute indices")?;
    if explicit_element_count != index_count {
        return Err(GeometryAuthoringSemanticError::SparseValueCount {
            attribute: attribute_index,
            index_count: indices.len(),
            value_elements: explicit_element_count_usize,
        }
        .into());
    }

    check_attribute_cardinality(
        attribute_index,
        attribute.domain(),
        logical_count,
        expected_cardinality,
    )?;

    let canonical_scalar_count = checked_sum(
        [
            usize_to_u64(values.len(), "sparse attribute values")?,
            usize_to_u64(default.len(), "sparse attribute default")?,
        ],
        "sparse attribute scalar count",
    )?;
    let index_scalar_count = u64::from(index_count);
    let payload_block_count = block_for_len(values.len())
        .checked_add(block_for_len(default.len()))
        .and_then(|blocks| blocks.checked_add(block_for_len(indices.len())))
        .ok_or(GeometryAuthoringSemanticError::AccountingOverflow {
            dimension: "attribute payload block count",
        })?;
    match scalar_kind {
        GeometryAttributeScalarKind::F32 => {
            context.add_payload(
                canonical_scalar_count,
                index_scalar_count,
                payload_block_count,
            )?;
        }
        GeometryAttributeScalarKind::U32 => {
            let u32_scalars = canonical_scalar_count
                .checked_add(index_scalar_count)
                .ok_or(GeometryAuthoringSemanticError::AccountingOverflow {
                    dimension: "u32 scalar count",
                })?;
            context.add_payload(0, u32_scalars, payload_block_count)?;
        }
    }

    T::inspect(attribute_index, values, 0, context)?;
    T::inspect(attribute_index, default, values.len(), context)?;
    let mut previous = None;
    for (element, &index) in indices.iter().enumerate() {
        context.work(SceneValidationWork::PayloadCell, 1)?;
        if index >= logical_count {
            return Err(GeometryAuthoringSemanticError::SparseIndexOutOfRange {
                attribute: attribute_index,
                element,
                index,
                logical_count,
            }
            .into());
        }
        if previous.is_some_and(|previous_index| index <= previous_index) {
            return Err(GeometryAuthoringSemanticError::SparseIndexOrder {
                attribute: attribute_index,
                element,
            }
            .into());
        }
        previous = Some(index);
    }
    Ok(ValidatedGeometryAttribute {
        input_index: attribute_index,
        scalar_kind,
        payload_kind: GeometryAttributePayloadKind::Sparse,
        source_element_count: explicit_element_count,
        element_count: logical_count,
        explicit_element_count,
        canonical_scalar_count,
        payload_block_count,
    })
}

fn attribute_element_count(
    attribute_index: usize,
    attribute: &GeometryAttributeAuthoring<'_>,
) -> Result<u32, GeometryAuthoringSemanticError> {
    match attribute.payload() {
        GeometryAttributePayloadAuthoring::DenseF32 {
            lanes,
            source_values,
            source_indices,
        } => {
            let source_count = scalar_element_count(
                attribute_index,
                lanes,
                source_values.len(),
                "f32 attribute source elements",
            )?;
            match source_indices {
                Presence::Absent => Ok(source_count),
                Presence::Present(indices) => {
                    collection_count(indices.len(), "attribute source projection elements")
                }
            }
        }
        GeometryAttributePayloadAuthoring::DenseU32 { lanes, values } => scalar_element_count(
            attribute_index,
            lanes,
            values.len(),
            "u32 attribute elements",
        ),
        GeometryAttributePayloadAuthoring::SparseF32 { logical_count, .. }
        | GeometryAttributePayloadAuthoring::SparseU32 { logical_count, .. } => Ok(logical_count),
    }
}

fn scalar_element_count(
    attribute: usize,
    lanes: u8,
    scalar_count: usize,
    collection: &'static str,
) -> Result<u32, GeometryAuthoringSemanticError> {
    if lanes == 0 {
        return Err(GeometryAuthoringSemanticError::ZeroAttributeLanes { attribute });
    }
    let lane_count = usize::from(lanes);
    if !scalar_count.is_multiple_of(lane_count) {
        return Err(GeometryAuthoringSemanticError::AttributeScalarShape { attribute, lanes });
    }
    collection_count(scalar_count / lane_count, collection)
}

fn validate_attribute_schema(
    attribute_index: usize,
    attribute: &GeometryAttributeAuthoring<'_>,
    scalar_kind: GeometryAttributeScalarKind,
    lanes: u8,
) -> Result<(), GeometryAuthoringSemanticError> {
    let kind = attribute.semantic().kind();
    if kind == AttributeSemanticKind::Custom {
        return Err(GeometryAuthoringSemanticError::CustomAttributeUnsupported {
            attribute: attribute_index,
        });
    }
    if attribute.domain() == AttributeDomain::Instance {
        return Err(GeometryAuthoringSemanticError::AttributeSchema {
            attribute: attribute_index,
        });
    }

    let valid = match kind {
        AttributeSemanticKind::Position => {
            attribute.domain() == AttributeDomain::Vertex
                && scalar_kind == GeometryAttributeScalarKind::F32
                && lanes == 3
        }
        AttributeSemanticKind::Normal => {
            matches!(
                attribute.domain(),
                AttributeDomain::Vertex | AttributeDomain::Corner
            ) && scalar_kind == GeometryAttributeScalarKind::F32
                && lanes == 3
        }
        AttributeSemanticKind::Tangent => {
            matches!(
                attribute.domain(),
                AttributeDomain::Vertex | AttributeDomain::Corner
            ) && scalar_kind == GeometryAttributeScalarKind::F32
                && lanes == 4
        }
        AttributeSemanticKind::Texcoord => {
            matches!(
                attribute.domain(),
                AttributeDomain::Vertex | AttributeDomain::Corner
            ) && scalar_kind == GeometryAttributeScalarKind::F32
                && (1..=3).contains(&lanes)
        }
        AttributeSemanticKind::Color => {
            matches!(
                attribute.domain(),
                AttributeDomain::Vertex | AttributeDomain::Corner | AttributeDomain::Face
            ) && scalar_kind == GeometryAttributeScalarKind::F32
                && matches!(lanes, 3 | 4)
        }
        AttributeSemanticKind::SmoothingGroup => {
            attribute.domain() == AttributeDomain::Face
                && scalar_kind == GeometryAttributeScalarKind::U32
                && lanes == 1
        }
        AttributeSemanticKind::Custom => false,
    };
    if valid {
        Ok(())
    } else {
        Err(GeometryAuthoringSemanticError::AttributeSchema {
            attribute: attribute_index,
        })
    }
}

fn inspect_f32_values<C>(
    attribute_index: usize,
    values: &[f32],
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<(), C>
where
    C: SceneValidationControl,
{
    inspect_f32_values_from(attribute_index, values, 0, context)
}

fn inspect_f32_values_from<C>(
    attribute_index: usize,
    values: &[f32],
    scalar_offset: usize,
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<(), C>
where
    C: SceneValidationControl,
{
    for (relative_scalar, value) in values.iter().enumerate() {
        context.work(SceneValidationWork::PayloadCell, 1)?;
        if !value.is_finite() {
            let scalar = scalar_offset.checked_add(relative_scalar).ok_or(
                GeometryAuthoringSemanticError::AccountingOverflow {
                    dimension: "attribute scalar index",
                },
            )?;
            return Err(GeometryAuthoringSemanticError::NonFiniteAttributeValue {
                attribute: attribute_index,
                scalar,
            }
            .into());
        }
    }
    Ok(())
}

fn inspect_u32_values<C>(
    values: &[u32],
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<(), C>
where
    C: SceneValidationControl,
{
    for _ in values {
        context.work(SceneValidationWork::PayloadCell, 1)?;
    }
    Ok(())
}

fn check_attribute_cardinality(
    attribute: usize,
    domain: AttributeDomain,
    actual: u32,
    expected: Option<u32>,
) -> Result<(), GeometryAuthoringSemanticError> {
    match expected {
        Some(expected) if actual != expected => {
            Err(GeometryAuthoringSemanticError::AttributeCardinality {
                attribute,
                domain,
                expected,
                actual,
            })
        }
        _ => Ok(()),
    }
}

fn checked_scalar_count(
    element_count: u32,
    lanes: u8,
) -> Result<u64, GeometryAuthoringSemanticError> {
    u64::from(element_count)
        .checked_mul(u64::from(lanes))
        .ok_or(GeometryAuthoringSemanticError::AccountingOverflow {
            dimension: "attribute scalar count",
        })
}

fn geometry_structural_bytes<C>(
    input: GeometryAuthoringBatch<'_>,
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<u64, C>
where
    C: SceneValidationControl,
{
    let logical_mesh_count = collection_count(input.logical_meshes().len(), "logical meshes")?;
    let primitive_count = collection_count(input.primitives().len(), "primitives")?;
    let attribute_count = collection_count(input.attributes().len(), "attributes")?;
    let mesh_instance_count = collection_count(input.mesh_instances().len(), "mesh instances")?;
    let directory_reference_bytes = checked_sum_bytes(
        [
            reference_slot_bytes(logical_mesh_count != 0),
            reference_slot_bytes(primitive_count != 0),
            reference_slot_bytes(attribute_count != 0),
            reference_slot_bytes(mesh_instance_count != 0),
        ],
        "canonical geometry directory references",
    )?;
    let mut primitive_bytes = 0_u64;
    for primitive in input.primitives() {
        context.work(SceneValidationWork::Traversal, 1)?;
        let boundary_block = matches!(
            primitive.topology(),
            GeometryTopologyAuthoring::Polygons(boundaries) if !boundaries.is_empty()
        );
        let index_block = matches!(
            primitive.indices(),
            Presence::Present(indices) if !indices.is_empty()
        );
        let reference_bytes = checked_sum_bytes(
            [
                reference_slot_bytes(boundary_block),
                reference_slot_bytes(index_block),
                reference_slot_bytes(matches!(primitive.material(), Presence::Present(_))),
                reference_slot_bytes(primitive.attributes().count() != 0),
            ],
            "canonical primitive references",
        )?;
        primitive_bytes = primitive_bytes
            .checked_add(PRIMITIVE_FIXED_BYTES)
            .and_then(|bytes| bytes.checked_add(reference_bytes))
            .ok_or(GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "canonical primitive records",
            })?;
    }
    let mut attribute_bytes = 0_u64;
    for attribute in input.attributes() {
        context.work(SceneValidationWork::Traversal, 1)?;
        let (values, sparse_indices, sparse_default) = match attribute.payload() {
            GeometryAttributePayloadAuthoring::DenseF32 {
                source_values,
                source_indices,
                ..
            } => (
                match source_indices {
                    Presence::Absent => !source_values.is_empty(),
                    Presence::Present(indices) => !indices.is_empty(),
                },
                false,
                false,
            ),
            GeometryAttributePayloadAuthoring::DenseU32 { values, .. } => {
                (!values.is_empty(), false, false)
            }
            GeometryAttributePayloadAuthoring::SparseF32 {
                indices,
                values,
                default,
                ..
            } => (!values.is_empty(), !indices.is_empty(), !default.is_empty()),
            GeometryAttributePayloadAuthoring::SparseU32 {
                indices,
                values,
                default,
                ..
            } => (!values.is_empty(), !indices.is_empty(), !default.is_empty()),
        };
        let reference_bytes = checked_sum_bytes(
            [
                reference_slot_bytes(values),
                reference_slot_bytes(sparse_indices),
                reference_slot_bytes(sparse_default),
            ],
            "canonical attribute references",
        )?;
        attribute_bytes = attribute_bytes
            .checked_add(ATTRIBUTE_FIXED_BYTES)
            .and_then(|bytes| bytes.checked_add(reference_bytes))
            .ok_or(GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "canonical attribute records",
            })?;
    }
    let structural_bytes = checked_sum_bytes(
        [
            GEOMETRY_DIRECTORY_FIXED_BYTES,
            directory_reference_bytes,
            checked_bytes(
                u64::from(logical_mesh_count),
                LOGICAL_MESH_ROW_BYTES,
                "canonical logical mesh records",
            )?,
            primitive_bytes,
            checked_bytes(
                u64::from(mesh_instance_count),
                MESH_INSTANCE_ROW_BYTES,
                "canonical mesh instance records",
            )?,
            attribute_bytes,
        ],
        "canonical geometry structural footprint",
    )?;
    Ok(structural_bytes)
}

const fn reference_slot_bytes(has_reference: bool) -> u64 {
    if has_reference {
        LOGICAL_REFERENCE_BYTES
    } else {
        LOGICAL_U8_BYTES
    }
}

fn checked_bytes(
    count: u64,
    width: u64,
    dimension: &'static str,
) -> Result<u64, GeometryAuthoringSemanticError> {
    count
        .checked_mul(width)
        .ok_or(GeometryAuthoringSemanticError::AccountingOverflow { dimension })
}

fn scratch_envelope(
    plan: CapacityPlan,
    resource: &'static str,
) -> Result<u64, GeometryAuthoringSemanticError> {
    plan.envelope_delta_bytes()
        .map_err(|cause| GeometryAuthoringSemanticError::ScratchCapacity { resource, cause })
}

fn checked_sum<const N: usize>(
    values: [u64; N],
    dimension: &'static str,
) -> Result<u64, GeometryAuthoringSemanticError> {
    values.into_iter().try_fold(0_u64, |total, value| {
        total
            .checked_add(value)
            .ok_or(GeometryAuthoringSemanticError::AccountingOverflow { dimension })
    })
}

fn checked_sum_bytes<const N: usize>(
    values: [u64; N],
    dimension: &'static str,
) -> Result<u64, GeometryAuthoringSemanticError> {
    checked_sum(values, dimension)
}

fn checked_range(
    range: AuthoringRange,
    table_len: usize,
) -> Result<Range<usize>, GeometryAuthoringSemanticError> {
    let start = u32_to_usize(range.offset(), "authoring range offset")?;
    let count = u32_to_usize(range.count(), "authoring range count")?;
    let end =
        start
            .checked_add(count)
            .ok_or(GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "authoring range end",
            })?;
    if end > table_len {
        return Err(GeometryAuthoringSemanticError::AccountingOverflow {
            dimension: "validated authoring range",
        });
    }
    Ok(start..end)
}

fn collection_count(
    count: usize,
    collection: &'static str,
) -> Result<u32, GeometryAuthoringSemanticError> {
    u32::try_from(count)
        .map_err(|_| GeometryAuthoringSemanticError::CountDomainTooLarge { collection })
}

fn usize_to_u64(
    value: usize,
    dimension: &'static str,
) -> Result<u64, GeometryAuthoringSemanticError> {
    u64::try_from(value)
        .map_err(|_| GeometryAuthoringSemanticError::AccountingOverflow { dimension })
}

fn u32_to_usize(
    value: u32,
    dimension: &'static str,
) -> Result<usize, GeometryAuthoringSemanticError> {
    usize::try_from(value)
        .map_err(|_| GeometryAuthoringSemanticError::AccountingOverflow { dimension })
}

fn block_for_len(len: usize) -> u32 {
    u32::from(len != 0)
}

fn block_for_u64(len: u64) -> u32 {
    u32::from(len != 0)
}

fn enforce_limit<E>(
    limit: GeometryAuthoringLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), GeometryAuthoringFailure<E>>
where
    E: Error + Send + Sync + 'static,
{
    if actual <= maximum {
        Ok(())
    } else {
        Err(GeometryAuthoringSemanticError::LimitExceeded {
            limit,
            actual,
            maximum,
        }
        .into())
    }
}
