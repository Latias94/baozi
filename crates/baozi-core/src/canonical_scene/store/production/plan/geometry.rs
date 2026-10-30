#![allow(
    clippy::result_large_err,
    reason = "geometry authoring failures retain bounded control evidence inline without a failure-path allocation"
)]

use std::error::Error;
use std::fmt;
use std::ops::Range;

use baozi_operation::{CapacityPlan, CapacityPlanError};
use thiserror::Error;

use crate::attribute::{AttributeDomain, AttributeSemantic, AttributeSemanticKind};
use crate::canonical_scene::store::typed_storage::build::GeometryStoragePlan;
use crate::entity::validate_entity_name_str;
use crate::geometry::TopologyKind;
use crate::{SCENE_VALIDATION_POLL_INTERVAL, SceneValidationLimits, SceneValidationWork};

use super::super::control::CanonicalAuthoringControl;
use super::super::input::Presence;
use super::super::input::appearance::MaterialLocalId;
use super::super::input::geometry::{
    AuthoringRange, GeometryAttributeAuthoring, GeometryAttributePayloadAuthoring,
    GeometryAuthoringBatch, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
};

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
    #[error("logical mesh {logical_mesh} has an invalid entity name")]
    InvalidLogicalMeshName {
        logical_mesh: usize,
        #[source]
        source: crate::entity::EntityNameError,
    },
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
    #[error("attribute {attribute} has non-finite floating-point source value at scalar {scalar}")]
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
    F64,
    U32,
    I32,
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
    semantic_name_offset: u32,
    semantic_name_count: u32,
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

    pub(in crate::canonical_scene::store) const fn semantic_name_offset(&self) -> u32 {
        self.semantic_name_offset
    }

    pub(in crate::canonical_scene::store) const fn semantic_name_count(&self) -> u32 {
        self.semantic_name_count
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

    pub(in crate::canonical_scene::store) const fn semantic_name_offset(&self) -> u32 {
        self.plan.semantic_name_offset()
    }

    pub(in crate::canonical_scene::store) const fn semantic_name_count(&self) -> u32 {
        self.plan.semantic_name_count()
    }

    pub(in crate::canonical_scene::store) const fn authored_payload(
        &self,
    ) -> GeometryAttributePayloadAuthoring<'input> {
        self.input.payload()
    }

    pub(in crate::canonical_scene::store) const fn lanes(&self) -> u8 {
        match self.input.payload() {
            GeometryAttributePayloadAuthoring::DenseF32 { lanes, .. }
            | GeometryAttributePayloadAuthoring::DenseF64 { lanes, .. }
            | GeometryAttributePayloadAuthoring::DenseU32 { lanes, .. }
            | GeometryAttributePayloadAuthoring::DenseI32 { lanes, .. }
            | GeometryAttributePayloadAuthoring::SparseF32 { lanes, .. }
            | GeometryAttributePayloadAuthoring::SparseU32 { lanes, .. }
            | GeometryAttributePayloadAuthoring::SparseI32 { lanes, .. } => lanes,
        }
    }
}

#[must_use = "dropping the validated geometry batch releases its compiled-plan reservation"]
pub(in crate::canonical_scene::store) struct ValidatedGeometryBatch<'input, R> {
    compiled: GeometryValidationStorage<R>,
    input: GeometryAuthoringBatch<'input>,
}

struct GeometryValidationStorage<R> {
    logical_mesh_count: u32,
    logical_mesh_name_byte_count: u32,
    attribute_semantic_name_byte_count: u32,
    primitive_count: u32,
    mesh_instance_count: u32,
    attribute_count: u32,
    f32_scalar_count: u64,
    f64_scalar_count: u64,
    u32_scalar_count: u64,
    i32_scalar_count: u64,
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
    const fn new(
        input: GeometryAuthoringBatch<'input>,
        compiled: GeometryValidationStorage<R>,
    ) -> Self {
        Self { compiled, input }
    }

    pub(in crate::canonical_scene::store) const fn input(&self) -> GeometryAuthoringBatch<'input> {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn logical_mesh_count(&self) -> u32 {
        self.compiled.logical_mesh_count
    }

    pub(in crate::canonical_scene::store) const fn logical_mesh_name_byte_count(&self) -> u32 {
        self.compiled.logical_mesh_name_byte_count
    }

    pub(in crate::canonical_scene::store) const fn attribute_semantic_name_byte_count(
        &self,
    ) -> u32 {
        self.compiled.attribute_semantic_name_byte_count
    }

    pub(in crate::canonical_scene::store) const fn primitive_count(&self) -> u32 {
        self.compiled.primitive_count
    }

    pub(in crate::canonical_scene::store) const fn mesh_instance_count(&self) -> u32 {
        self.compiled.mesh_instance_count
    }

    pub(in crate::canonical_scene::store) const fn attribute_count(&self) -> u32 {
        self.compiled.attribute_count
    }

    pub(in crate::canonical_scene::store) const fn f32_scalar_count(&self) -> u64 {
        self.compiled.f32_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn f64_scalar_count(&self) -> u64 {
        self.compiled.f64_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn u32_scalar_count(&self) -> u64 {
        self.compiled.u32_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn i32_scalar_count(&self) -> u64 {
        self.compiled.i32_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn payload_block_count(&self) -> u32 {
        self.compiled.payload_block_count
    }

    pub(in crate::canonical_scene::store) const fn entity_count(&self) -> u64 {
        self.compiled.entity_count
    }

    pub(in crate::canonical_scene::store) const fn edge_count(&self) -> u64 {
        self.compiled.edge_count
    }

    pub(in crate::canonical_scene::store) const fn reference_count(&self) -> u64 {
        self.compiled.reference_count
    }

    pub(in crate::canonical_scene::store) const fn semantic_cell_count(&self) -> u64 {
        self.compiled.semantic_cell_count
    }

    pub(in crate::canonical_scene::store) const fn canonical_bytes(&self) -> u64 {
        self.compiled.canonical_bytes
    }

    pub(in crate::canonical_scene::store) const fn validation_work(&self) -> u64 {
        self.compiled.validation_work
    }

    pub(in crate::canonical_scene::store) const fn scratch_bytes(&self) -> u64 {
        self.compiled.scratch_bytes
    }

    pub(in crate::canonical_scene::store) fn primitive(
        &self,
        index: usize,
    ) -> Option<ValidatedGeometryPrimitive<'_, 'input>> {
        let plan = self.compiled.primitive_plans.get(index)?;
        let input = *self.input.primitives().get(plan.input_index)?;
        let range =
            checked_range(plan.attribute_plans, self.compiled.attribute_plans.len()).ok()?;
        Some(ValidatedGeometryPrimitive {
            input,
            plan,
            attributes: self.compiled.attribute_plans.get(range.clone())?,
            input_attributes: self.input.attributes(),
        })
    }

    pub(in crate::canonical_scene::store) fn attribute(
        &self,
        index: usize,
    ) -> Option<ValidatedGeometryAttributeView<'_, 'input>> {
        let plan = self.compiled.attribute_plans.get(index)?;
        let input = self.input.attributes().get(plan.input_index)?;
        Some(ValidatedGeometryAttributeView { input, plan })
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_storage_drop_probe(
        &mut self,
        stage: std::rc::Rc<std::cell::Cell<u8>>,
    ) {
        self.compiled.storage_drop_probe = Some(GeometryStorageDropProbe(stage));
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store::production) fn into_compiled_facts_for_test(
        self,
    ) -> (u32, u32, u32, u32, usize, usize, R) {
        let GeometryValidationStorage {
            logical_mesh_count,
            primitive_count,
            mesh_instance_count,
            attribute_count,
            primitive_plans,
            attribute_plans,
            reservation,
            ..
        } = self.compiled;
        (
            logical_mesh_count,
            primitive_count,
            mesh_instance_count,
            attribute_count,
            primitive_plans.len(),
            attribute_plans.len(),
            reservation,
        )
    }
}

impl<R> fmt::Debug for ValidatedGeometryBatch<'_, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedGeometryBatch")
            .field("logical_mesh_count", &self.compiled.logical_mesh_count)
            .field(
                "logical_mesh_name_byte_count",
                &self.compiled.logical_mesh_name_byte_count,
            )
            .field(
                "attribute_semantic_name_byte_count",
                &self.compiled.attribute_semantic_name_byte_count,
            )
            .field("primitive_count", &self.compiled.primitive_count)
            .field("mesh_instance_count", &self.compiled.mesh_instance_count)
            .field("attribute_count", &self.compiled.attribute_count)
            .field("f32_scalar_count", &self.compiled.f32_scalar_count)
            .field("f64_scalar_count", &self.compiled.f64_scalar_count)
            .field("u32_scalar_count", &self.compiled.u32_scalar_count)
            .field("i32_scalar_count", &self.compiled.i32_scalar_count)
            .field("payload_block_count", &self.compiled.payload_block_count)
            .field("entity_count", &self.compiled.entity_count)
            .field("edge_count", &self.compiled.edge_count)
            .field("reference_count", &self.compiled.reference_count)
            .field("semantic_cell_count", &self.compiled.semantic_cell_count)
            .field("canonical_bytes", &self.compiled.canonical_bytes)
            .field("validation_work", &self.compiled.validation_work)
            .field("scratch_bytes", &self.compiled.scratch_bytes)
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

struct GeometryValidationScratch<R> {
    primitive_plans: Vec<ValidatedGeometryPrimitivePlan>,
    attribute_plans: Vec<ValidatedGeometryAttribute>,
    // Every allocation above must drop before its accounting token.
    reservation: R,
}

type GeometryValidationResult<T, C> =
    Result<T, GeometryAuthoringFailure<<C as CanonicalAuthoringControl>::Error>>;

struct GeometryValidationContext<'control, C: CanonicalAuthoringControl> {
    control: &'control mut C,
    limits: SceneValidationLimits,
    work: u64,
    work_since_poll: u64,
    edges: u64,
    references: u64,
    semantic_cells: u64,
    f32_scalars: u64,
    f64_scalars: u64,
    u32_scalars: u64,
    i32_scalars: u64,
    payload_blocks: u32,
    canonical_bytes: u64,
    rows: [u64; 4],
    name_bytes: u64,
}

impl<'control, C: CanonicalAuthoringControl> GeometryValidationContext<'control, C> {
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
            f64_scalars: 0,
            u32_scalars: 0,
            i32_scalars: 0,
            payload_blocks: 0,
            canonical_bytes: 0,
            rows: [0; 4],
            name_bytes: 0,
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

    fn run_unit<R>(
        &mut self,
        kind: SceneValidationWork,
        action: impl FnMut() -> R,
    ) -> GeometryValidationResult<R, C> {
        let final_work =
            self.work
                .checked_add(1)
                .ok_or(GeometryAuthoringSemanticError::AccountingOverflow {
                    dimension: "validation work",
                })?;
        enforce_limit::<C::Error>(
            GeometryAuthoringLimit::Work,
            final_work,
            self.limits.max_work(),
        )?;

        let result = self
            .control
            .run_unit(kind, action)
            .map_err(GeometryAuthoringFailure::Control)?;
        self.work = final_work;
        self.work_since_poll += 1;
        if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
            self.poll_now()?;
        }
        Ok(result)
    }

    fn add_relations(&mut self, amount: u64) -> GeometryValidationResult<(), C> {
        for _ in 0..amount {
            let current_edges = self.edges;
            let max_edges = self.limits.max_edges();
            let final_edges = self.run_unit(SceneValidationWork::GraphEdge, || {
                let final_edges = current_edges.checked_add(1).ok_or(
                    GeometryAuthoringSemanticError::AccountingOverflow { dimension: "edges" },
                )?;
                if final_edges > max_edges {
                    return Err(GeometryAuthoringSemanticError::LimitExceeded {
                        limit: GeometryAuthoringLimit::Edges,
                        actual: final_edges,
                        maximum: max_edges,
                    });
                }
                Ok::<u64, GeometryAuthoringSemanticError>(final_edges)
            })??;
            self.edges = final_edges;

            let current_references = self.references;
            let max_references = self.limits.max_references();
            let final_references = self.run_unit(SceneValidationWork::Reference, || {
                let final_references = current_references.checked_add(1).ok_or(
                    GeometryAuthoringSemanticError::AccountingOverflow {
                        dimension: "references",
                    },
                )?;
                if final_references > max_references {
                    return Err(GeometryAuthoringSemanticError::LimitExceeded {
                        limit: GeometryAuthoringLimit::References,
                        actual: final_references,
                        maximum: max_references,
                    });
                }
                Ok::<u64, GeometryAuthoringSemanticError>(final_references)
            })??;
            self.references = final_references;
        }
        Ok(())
    }

    fn add_payload(
        &mut self,
        f32_scalars: u64,
        f64_scalars: u64,
        u32_scalars: u64,
        i32_scalars: u64,
        payload_blocks: u32,
    ) -> GeometryValidationResult<(), C> {
        let added_cells = f32_scalars
            .checked_add(f64_scalars)
            .and_then(|count| count.checked_add(u32_scalars))
            .and_then(|count| count.checked_add(i32_scalars))
            .ok_or(GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "semantic cells",
            })?;
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
        let final_f64 = self.f64_scalars.checked_add(f64_scalars).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "f64 scalar count",
            },
        )?;
        let final_u32 = self.u32_scalars.checked_add(u32_scalars).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "u32 scalar count",
            },
        )?;
        let final_i32 = self.i32_scalars.checked_add(i32_scalars).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "i32 scalar count",
            },
        )?;
        let final_blocks = self.payload_blocks.checked_add(payload_blocks).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "payload block count",
            },
        )?;
        let final_canonical_bytes = GeometryStoragePlan::canonical_bytes_for_shape(
            self.rows,
            self.name_bytes,
            [final_f32, final_f64, final_u32, final_i32],
        )
        .map_err(|_| GeometryAuthoringSemanticError::AccountingOverflow {
            dimension: "initialized typed geometry footprint",
        })?;
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
        self.f64_scalars = final_f64;
        self.u32_scalars = final_u32;
        self.i32_scalars = final_i32;
        self.payload_blocks = final_blocks;
        self.canonical_bytes = final_canonical_bytes;
        Ok(())
    }

    fn add_utf8_payload(&mut self, bytes: u64) -> GeometryValidationResult<(), C> {
        let final_cells = self.semantic_cells.checked_add(bytes).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "semantic cells",
            },
        )?;
        let final_name_bytes = self.name_bytes.checked_add(bytes).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "geometry name bytes",
            },
        )?;
        let final_canonical_bytes = GeometryStoragePlan::canonical_bytes_for_shape(
            self.rows,
            final_name_bytes,
            [
                self.f32_scalars,
                self.f64_scalars,
                self.u32_scalars,
                self.i32_scalars,
            ],
        )
        .map_err(|_| GeometryAuthoringSemanticError::AccountingOverflow {
            dimension: "initialized typed geometry footprint",
        })?;
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
        self.canonical_bytes = final_canonical_bytes;
        self.name_bytes = final_name_bytes;
        Ok(())
    }

    fn add_payload_blocks(&mut self, blocks: u32) -> GeometryValidationResult<(), C> {
        self.payload_blocks = self.payload_blocks.checked_add(blocks).ok_or(
            GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "payload block count",
            },
        )?;
        Ok(())
    }

    fn set_shape(&mut self, rows: [u64; 4]) -> GeometryValidationResult<(), C> {
        let bytes =
            GeometryStoragePlan::canonical_bytes_for_shape(rows, 0, [0; 4]).map_err(|_| {
                GeometryAuthoringSemanticError::AccountingOverflow {
                    dimension: "initialized typed geometry rows",
                }
            })?;
        enforce_limit::<C::Error>(
            GeometryAuthoringLimit::CanonicalBytes,
            bytes,
            self.limits.max_canonical_bytes(),
        )?;
        self.canonical_bytes = bytes;
        self.rows = rows;
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
            .reserve_scratch(bytes)
            .map_err(GeometryAuthoringFailure::Control)?;
        self.control
            .validate_scratch(&reservation)
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
            .resize_scratch(reservation, bytes)
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

pub(in crate::canonical_scene::store) fn validate_geometry_batch_with_control<'input, C>(
    input: GeometryAuthoringBatch<'input>,
    authored_node_count: usize,
    appearance_material_count: u32,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedGeometryBatch<'input, C::Reservation>, GeometryAuthoringFailure<C::Error>>
where
    C: CanonicalAuthoringControl,
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
    C: CanonicalAuthoringControl,
{
    let mut context = GeometryValidationContext::new(limits, control);
    context.poll_now()?;

    // Admit observation of the fixed top-level shape before reserving input-sized scratch.
    // The typed footprint needs these lengths; it does not need a second descriptor scan.
    let (logical_mesh_count, primitive_count, attribute_count, mesh_instance_count) = context
        .run_unit(SceneValidationWork::Traversal, || {
            Ok::<_, GeometryAuthoringSemanticError>((
                collection_count(input.logical_meshes().len(), "logical meshes")?,
                collection_count(input.primitives().len(), "primitives")?,
                collection_count(input.attributes().len(), "attributes")?,
                collection_count(input.mesh_instances().len(), "mesh instances")?,
            ))
        })??;
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
    context.set_shape([
        u64::from(logical_mesh_count),
        u64::from(primitive_count),
        u64::from(attribute_count),
        u64::from(mesh_instance_count),
    ])?;

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
    let mut storage = GeometryValidationScratch {
        primitive_plans,
        attribute_plans,
        reservation,
    };

    let mut expected_primitive_offset = 0_u32;
    let mut logical_mesh_name_offset = 0_u32;
    for (logical_mesh_index, logical_mesh) in input.logical_meshes().iter().copied().enumerate() {
        let (range, end, name, next_name_offset) =
            context.run_unit(SceneValidationWork::Entity, || {
                let range = logical_mesh.primitives();
                if range.count() == 0 {
                    return Err(GeometryAuthoringSemanticError::EmptyLogicalMesh {
                        logical_mesh: logical_mesh_index,
                    });
                }
                if range.offset() != expected_primitive_offset {
                    return Err(GeometryAuthoringSemanticError::LogicalMeshRangeOrder {
                        logical_mesh: logical_mesh_index,
                        expected: expected_primitive_offset,
                        actual: range.offset(),
                    });
                }
                let Some(end) = range.offset().checked_add(range.count()) else {
                    return Err(
                        GeometryAuthoringSemanticError::LogicalMeshRangeOutOfBounds {
                            logical_mesh: logical_mesh_index,
                            primitive_count: input.primitives().len(),
                        },
                    );
                };
                if end > primitive_count {
                    return Err(
                        GeometryAuthoringSemanticError::LogicalMeshRangeOutOfBounds {
                            logical_mesh: logical_mesh_index,
                            primitive_count: input.primitives().len(),
                        },
                    );
                }
                let name = match logical_mesh.name() {
                    Presence::Absent => None,
                    Presence::Present(name) => {
                        validate_entity_name_str(name).map_err(|source| {
                            GeometryAuthoringSemanticError::InvalidLogicalMeshName {
                                logical_mesh: logical_mesh_index,
                                source,
                            }
                        })?;
                        Some(name)
                    }
                };
                let name_count = match name {
                    Some(name) => collection_count(name.len(), "logical mesh name bytes")?,
                    None => 0,
                };
                let next_name_offset = logical_mesh_name_offset.checked_add(name_count).ok_or(
                    GeometryAuthoringSemanticError::AccountingOverflow {
                        dimension: "logical mesh name byte offset",
                    },
                )?;
                Ok((range, end, name, next_name_offset))
            })??;
        if let Some(name) = name {
            for byte in name.as_bytes() {
                context.run_unit(SceneValidationWork::PayloadCell, || *byte)?;
            }
            context.add_utf8_payload(usize_to_u64(name.len(), "logical mesh name bytes")?)?;
        }
        context.add_relations(u64::from(range.count()))?;
        expected_primitive_offset = end;
        logical_mesh_name_offset = next_name_offset;
    }
    context.add_payload_blocks(block_for_u64(u64::from(logical_mesh_name_offset)))?;
    context.run_unit(SceneValidationWork::Traversal, || {
        if expected_primitive_offset == primitive_count {
            return Ok(());
        }
        let unowned = primitive_count
            .checked_sub(expected_primitive_offset)
            .ok_or(GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "unowned primitive count",
            })?;
        Err(GeometryAuthoringSemanticError::UnownedPrimitives {
            unowned: u32_to_usize(unowned, "unowned primitive count")?,
        })
    })??;

    let mut expected_attribute_offset = 0_u32;
    let mut attribute_semantic_name_offset = 0_u32;
    for (primitive_index, primitive) in input.primitives().iter().copied().enumerate() {
        let (attribute_range, attribute_end, has_material, attribute_indices) =
            context.run_unit(SceneValidationWork::Entity, || {
                let attribute_range = primitive.attributes();
                if attribute_range.offset() != expected_attribute_offset {
                    return Err(
                        GeometryAuthoringSemanticError::PrimitiveAttributeRangeOrder {
                            primitive: primitive_index,
                            expected: expected_attribute_offset,
                            actual: attribute_range.offset(),
                        },
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
                        },
                    );
                };
                if attribute_end > attribute_count {
                    return Err(
                        GeometryAuthoringSemanticError::PrimitiveAttributeRangeOutOfBounds {
                            primitive: primitive_index,
                            attribute_count: input.attributes().len(),
                        },
                    );
                }

                let has_material = if let Presence::Present(material) = primitive.material() {
                    if material.index() >= appearance_material_count {
                        return Err(GeometryAuthoringSemanticError::MaterialOutOfRange {
                            primitive: primitive_index,
                            material: material.index(),
                            appearance_material_count,
                        });
                    }
                    true
                } else {
                    false
                };
                let attribute_indices = checked_range(attribute_range, input.attributes().len())?;
                Ok((
                    attribute_range,
                    attribute_end,
                    has_material,
                    attribute_indices,
                ))
            })??;
        context.add_relations(u64::from(attribute_range.count()))?;
        if has_material {
            context.add_relations(1)?;
        }

        let plan = validate_primitive(
            primitive_index,
            primitive,
            attribute_indices,
            input.attributes(),
            &mut storage.attribute_plans,
            &mut attribute_semantic_name_offset,
            &mut context,
        )?;
        storage.primitive_plans.push(plan);
        expected_attribute_offset = attribute_end;
    }
    context.run_unit(SceneValidationWork::Traversal, || {
        if expected_attribute_offset == attribute_count {
            return Ok(());
        }
        let unowned = attribute_count
            .checked_sub(expected_attribute_offset)
            .ok_or(GeometryAuthoringSemanticError::AccountingOverflow {
                dimension: "unowned attribute count",
            })?;
        Err(GeometryAuthoringSemanticError::UnownedAttributes {
            unowned: u32_to_usize(unowned, "unowned attribute count")?,
        })
    })??;
    context.add_payload_blocks(block_for_u64(u64::from(attribute_semantic_name_offset)))?;

    for (instance_index, instance) in input.mesh_instances().iter().copied().enumerate() {
        context.run_unit(SceneValidationWork::Entity, || {
            let owner = instance.owner().index();
            if u32_to_usize(owner, "mesh instance owner")? >= authored_node_count {
                return Err(GeometryAuthoringSemanticError::InstanceOwnerOutOfRange {
                    instance: instance_index,
                    owner,
                    node_count: authored_node_count,
                });
            }

            let logical_mesh = instance.logical_mesh().index();
            if logical_mesh >= logical_mesh_count {
                return Err(GeometryAuthoringSemanticError::InstanceMeshOutOfRange {
                    instance: instance_index,
                    logical_mesh,
                    logical_mesh_count: input.logical_meshes().len(),
                });
            }
            Ok(())
        })??;
        context.add_relations(1)?;
        context.add_relations(1)?;
    }

    let canonical_bytes = context.canonical_bytes;
    let validation_work = context.work;
    let GeometryValidationScratch {
        primitive_plans,
        attribute_plans,
        reservation,
    } = storage;
    let compiled = GeometryValidationStorage {
        logical_mesh_count,
        logical_mesh_name_byte_count: logical_mesh_name_offset,
        attribute_semantic_name_byte_count: attribute_semantic_name_offset,
        primitive_count,
        mesh_instance_count,
        attribute_count,
        f32_scalar_count: context.f32_scalars,
        f64_scalar_count: context.f64_scalars,
        u32_scalar_count: context.u32_scalars,
        i32_scalar_count: context.i32_scalars,
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
    let validated = ValidatedGeometryBatch::new(input, compiled);
    context.finish()?;
    Ok(validated)
}

fn validate_primitive<C>(
    primitive_index: usize,
    primitive: GeometryPrimitiveAuthoring<'_>,
    attribute_indices: Range<usize>,
    attributes: &[GeometryAttributeAuthoring<'_>],
    attribute_plans: &mut Vec<ValidatedGeometryAttribute>,
    attribute_semantic_name_offset: &mut u32,
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<ValidatedGeometryPrimitivePlan, C>
where
    C: CanonicalAuthoringControl,
{
    let mut previous_key: Option<(&AttributeSemantic, AttributeDomain)> = None;
    let mut position_index = None;
    let mut inferred_vertex_count = 0_u32;
    for attribute_index in attribute_indices.clone() {
        context.run_unit(SceneValidationWork::Entity, || {
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
                        });
                    }
                    std::cmp::Ordering::Greater => {
                        return Err(GeometryAuthoringSemanticError::AttributeOrder {
                            primitive: primitive_index,
                            attribute: attribute_index,
                        });
                    }
                    std::cmp::Ordering::Less => {}
                }
            }
            validate_attribute_authored_shape(attribute_index, attribute)?;
            if attribute.semantic().kind() == AttributeSemanticKind::Position {
                position_index = Some(attribute_index);
            }
            if attribute.domain() == AttributeDomain::Vertex {
                inferred_vertex_count =
                    inferred_vertex_count.max(attribute_element_count(attribute_index, attribute)?);
            }
            previous_key = Some(key);
            Ok(())
        })??;
    }

    let mut position_plan = match position_index {
        Some(attribute_index) => Some((
            attribute_index,
            validate_attribute(
                attribute_index,
                &attributes[attribute_index],
                None,
                attribute_semantic_name_offset,
                context,
            )?,
        )),
        None => None,
    };
    let vertex_count = if let Some((_, plan)) = &position_plan {
        plan.element_count()
    } else {
        infer_missing_position_vertex_count(primitive, inferred_vertex_count, context)?
    };
    context.run_unit(SceneValidationWork::Traversal, || {
        if vertex_count > 0 && position_plan.is_none() {
            return Err(GeometryAuthoringSemanticError::MissingPosition {
                primitive: primitive_index,
                vertex_count,
            });
        }
        Ok(())
    })??;

    let (corner_count, index_count) =
        validate_index_stream(primitive_index, primitive.indices(), vertex_count, context)?;
    let (topology, face_count) =
        validate_topology(primitive_index, primitive.topology(), corner_count, context)?;

    for attribute_index in attribute_indices {
        let expected = context.run_unit(SceneValidationWork::Traversal, || {
            let attribute = &attributes[attribute_index];
            let expected = domain_cardinality(
                attribute_index,
                attribute.domain(),
                vertex_count,
                corner_count,
                face_count,
            )?;
            validate_expected_attribute_cardinality(attribute_index, attribute, expected)?;
            Ok::<u32, GeometryAuthoringSemanticError>(expected)
        })??;
        let attribute = &attributes[attribute_index];
        let plan = match position_plan.take() {
            Some((position, plan)) if position == attribute_index => plan,
            Some((position, plan)) => {
                position_plan = Some((position, plan));
                validate_attribute(
                    attribute_index,
                    attribute,
                    Some(expected),
                    attribute_semantic_name_offset,
                    context,
                )?
            }
            None => validate_attribute(
                attribute_index,
                attribute,
                Some(expected),
                attribute_semantic_name_offset,
                context,
            )?,
        };
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
    C: CanonicalAuthoringControl,
{
    if let Presence::Present(indices) = primitive.indices() {
        for &index in indices {
            inferred = context.run_unit(SceneValidationWork::PayloadCell, || {
                let declared_count = index.checked_add(1).ok_or(
                    GeometryAuthoringSemanticError::CountDomainTooLarge {
                        collection: "primitive vertices",
                    },
                )?;
                Ok::<_, GeometryAuthoringSemanticError>(inferred.max(declared_count))
            })??;
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
    C: CanonicalAuthoringControl,
{
    match indices {
        Presence::Absent => Ok((vertex_count, 0)),
        Presence::Present(indices) => {
            let index_count = collection_count(indices.len(), "primitive indices")?;
            context.add_payload(
                0,
                0,
                u64::from(index_count),
                0,
                block_for_len(indices.len()),
            )?;
            for (corner, &index) in indices.iter().enumerate() {
                context.run_unit(SceneValidationWork::PayloadCell, || {
                    if index >= vertex_count {
                        return Err(GeometryAuthoringSemanticError::VertexIndexOutOfRange {
                            primitive: primitive_index,
                            corner,
                            index,
                            vertex_count,
                        });
                    }
                    Ok(())
                })??;
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
    C: CanonicalAuthoringControl,
{
    let validated = match topology {
        GeometryTopologyAuthoring::Points => context
            .run_unit(SceneValidationWork::Traversal, || {
                Ok::<_, GeometryAuthoringSemanticError>((TopologyKind::Points, corner_count))
            })??,
        GeometryTopologyAuthoring::LineList => {
            context.run_unit(SceneValidationWork::Traversal, || {
                Ok::<_, GeometryAuthoringSemanticError>((
                    TopologyKind::LineList,
                    list_face_count(primitive_index, TopologyKind::LineList, corner_count, 2)?,
                ))
            })??
        }
        GeometryTopologyAuthoring::LineStrip => {
            context.run_unit(SceneValidationWork::Traversal, || {
                Ok::<_, GeometryAuthoringSemanticError>((
                    TopologyKind::LineStrip,
                    corner_count.saturating_sub(1),
                ))
            })??
        }
        GeometryTopologyAuthoring::LineLoop => {
            context.run_unit(SceneValidationWork::Traversal, || {
                Ok::<_, GeometryAuthoringSemanticError>((
                    TopologyKind::LineLoop,
                    if corner_count < 2 { 0 } else { corner_count },
                ))
            })??
        }
        GeometryTopologyAuthoring::TriangleList => {
            context.run_unit(SceneValidationWork::Traversal, || {
                Ok::<_, GeometryAuthoringSemanticError>((
                    TopologyKind::TriangleList,
                    list_face_count(primitive_index, TopologyKind::TriangleList, corner_count, 3)?,
                ))
            })??
        }
        GeometryTopologyAuthoring::TriangleStrip => {
            context.run_unit(SceneValidationWork::Traversal, || {
                Ok::<_, GeometryAuthoringSemanticError>((
                    TopologyKind::TriangleStrip,
                    corner_count.saturating_sub(2),
                ))
            })??
        }
        GeometryTopologyAuthoring::TriangleFan => {
            context.run_unit(SceneValidationWork::Traversal, || {
                Ok::<_, GeometryAuthoringSemanticError>((
                    TopologyKind::TriangleFan,
                    corner_count.saturating_sub(2),
                ))
            })??
        }
        GeometryTopologyAuthoring::Polygons(boundaries) => {
            let face_count = collection_count(boundaries.len(), "polygon faces")?;
            context.add_payload(
                0,
                0,
                u64::from(face_count),
                0,
                block_for_len(boundaries.len()),
            )?;
            let mut boundary_corners = 0_u32;
            for (face, &face_corner_count) in boundaries.iter().enumerate() {
                boundary_corners = context.run_unit(SceneValidationWork::PayloadCell, || {
                    if face_corner_count < 3 {
                        return Err(GeometryAuthoringSemanticError::PolygonFaceTooSmall {
                            primitive: primitive_index,
                            face,
                            corner_count: face_corner_count,
                        });
                    }
                    boundary_corners.checked_add(face_corner_count).ok_or(
                        GeometryAuthoringSemanticError::PolygonCornerCountOverflow {
                            primitive: primitive_index,
                            face,
                        },
                    )
                })??;
            }
            context.run_unit(SceneValidationWork::Traversal, || {
                if boundary_corners != corner_count {
                    return Err(GeometryAuthoringSemanticError::PolygonCornerCountMismatch {
                        primitive: primitive_index,
                        boundary_corners,
                        corner_count,
                    });
                }
                Ok((TopologyKind::Polygons, face_count))
            })??
        }
    };
    Ok(validated)
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
    attribute_semantic_name_offset: &mut u32,
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<ValidatedGeometryAttribute, C>
where
    C: CanonicalAuthoringControl,
{
    let semantic_name_offset = *attribute_semantic_name_offset;
    let semantic_name_count = match attribute.semantic() {
        AttributeSemantic::Custom(custom) => {
            let name = custom.as_str();
            let count = collection_count(name.len(), "attribute semantic name bytes")?;
            let next = semantic_name_offset.checked_add(count).ok_or(
                GeometryAuthoringSemanticError::AccountingOverflow {
                    dimension: "attribute semantic name byte offset",
                },
            )?;
            for byte in name.as_bytes() {
                context.run_unit(SceneValidationWork::PayloadCell, || *byte)?;
            }
            context.add_utf8_payload(usize_to_u64(name.len(), "attribute semantic name bytes")?)?;
            *attribute_semantic_name_offset = next;
            count
        }
        _ => 0,
    };
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
            context.add_payload(canonical_scalar_count, 0, 0, 0, payload_block_count)?;

            inspect_f32_values(attribute_index, source_values, context)?;
            if let Some(indices) = projection {
                for (element, &index) in indices.iter().enumerate() {
                    context.run_unit(SceneValidationWork::PayloadCell, || {
                        if index >= source_element_count {
                            return Err(
                                GeometryAuthoringSemanticError::SourceProjectionOutOfRange {
                                    attribute: attribute_index,
                                    element,
                                    index,
                                    source_count: source_element_count,
                                },
                            );
                        }
                        Ok(())
                    })??;
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
                semantic_name_offset,
                semantic_name_count,
            })
        }
        GeometryAttributePayloadAuthoring::DenseF64 { lanes, values } => {
            let element_count = scalar_element_count(
                attribute_index,
                lanes,
                values.len(),
                "f64 attribute elements",
            )?;
            validate_attribute_schema(
                attribute_index,
                attribute,
                GeometryAttributeScalarKind::F64,
                lanes,
            )?;
            check_attribute_cardinality(
                attribute_index,
                attribute.domain(),
                element_count,
                expected_cardinality,
            )?;
            let canonical_scalar_count = usize_to_u64(values.len(), "f64 attribute scalars")?;
            let payload_block_count = block_for_len(values.len());
            context.add_payload(0, canonical_scalar_count, 0, 0, payload_block_count)?;
            inspect_f64_values(attribute_index, values, context)?;
            Ok(ValidatedGeometryAttribute {
                input_index: attribute_index,
                scalar_kind: GeometryAttributeScalarKind::F64,
                payload_kind: GeometryAttributePayloadKind::Dense,
                source_element_count: element_count,
                element_count,
                explicit_element_count: element_count,
                canonical_scalar_count,
                payload_block_count,
                semantic_name_offset,
                semantic_name_count,
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
            context.add_payload(0, 0, canonical_scalar_count, 0, payload_block_count)?;
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
                semantic_name_offset,
                semantic_name_count,
            })
        }
        GeometryAttributePayloadAuthoring::DenseI32 { lanes, values } => {
            let element_count = scalar_element_count(
                attribute_index,
                lanes,
                values.len(),
                "i32 attribute elements",
            )?;
            validate_attribute_schema(
                attribute_index,
                attribute,
                GeometryAttributeScalarKind::I32,
                lanes,
            )?;
            check_attribute_cardinality(
                attribute_index,
                attribute.domain(),
                element_count,
                expected_cardinality,
            )?;
            let canonical_scalar_count = usize_to_u64(values.len(), "i32 attribute scalars")?;
            let payload_block_count = block_for_len(values.len());
            context.add_payload(0, 0, 0, canonical_scalar_count, payload_block_count)?;
            inspect_i32_values(values, context)?;
            Ok(ValidatedGeometryAttribute {
                input_index: attribute_index,
                scalar_kind: GeometryAttributeScalarKind::I32,
                payload_kind: GeometryAttributePayloadKind::Dense,
                source_element_count: element_count,
                element_count,
                explicit_element_count: element_count,
                canonical_scalar_count,
                payload_block_count,
                semantic_name_offset,
                semantic_name_count,
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
            semantic_name_offset,
            semantic_name_count,
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
            semantic_name_offset,
            semantic_name_count,
            context,
        ),
        GeometryAttributePayloadAuthoring::SparseI32 {
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
            semantic_name_offset,
            semantic_name_count,
            context,
        ),
    }
}

trait GeometryScalar: Copy {
    const KIND: GeometryAttributeScalarKind;

    fn inspect<C: CanonicalAuthoringControl>(
        attribute_index: usize,
        values: &[Self],
        scalar_offset: usize,
        context: &mut GeometryValidationContext<'_, C>,
    ) -> GeometryValidationResult<(), C>;
}

impl GeometryScalar for f32 {
    const KIND: GeometryAttributeScalarKind = GeometryAttributeScalarKind::F32;

    fn inspect<C: CanonicalAuthoringControl>(
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

    fn inspect<C: CanonicalAuthoringControl>(
        _attribute_index: usize,
        values: &[Self],
        _scalar_offset: usize,
        context: &mut GeometryValidationContext<'_, C>,
    ) -> GeometryValidationResult<(), C> {
        inspect_u32_values(values, context)
    }
}

impl GeometryScalar for i32 {
    const KIND: GeometryAttributeScalarKind = GeometryAttributeScalarKind::I32;

    fn inspect<C: CanonicalAuthoringControl>(
        _attribute_index: usize,
        values: &[Self],
        _scalar_offset: usize,
        context: &mut GeometryValidationContext<'_, C>,
    ) -> GeometryValidationResult<(), C> {
        inspect_i32_values(values, context)
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
    semantic_name_offset: u32,
    semantic_name_count: u32,
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<ValidatedGeometryAttribute, C>
where
    C: CanonicalAuthoringControl,
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
                0,
                index_scalar_count,
                0,
                payload_block_count,
            )?;
        }
        GeometryAttributeScalarKind::U32 => {
            let u32_scalars = canonical_scalar_count
                .checked_add(index_scalar_count)
                .ok_or(GeometryAuthoringSemanticError::AccountingOverflow {
                    dimension: "u32 scalar count",
                })?;
            context.add_payload(0, 0, u32_scalars, 0, payload_block_count)?;
        }
        GeometryAttributeScalarKind::I32 => {
            context.add_payload(
                0,
                0,
                index_scalar_count,
                canonical_scalar_count,
                payload_block_count,
            )?;
        }
        GeometryAttributeScalarKind::F64 => {
            return Err(GeometryAuthoringSemanticError::AttributeSchema {
                attribute: attribute_index,
            }
            .into());
        }
    }

    T::inspect(attribute_index, values, 0, context)?;
    T::inspect(attribute_index, default, values.len(), context)?;
    let mut previous = None;
    for (element, &index) in indices.iter().enumerate() {
        previous = context.run_unit(SceneValidationWork::PayloadCell, || {
            if index >= logical_count {
                return Err(GeometryAuthoringSemanticError::SparseIndexOutOfRange {
                    attribute: attribute_index,
                    element,
                    index,
                    logical_count,
                });
            }
            if previous.is_some_and(|previous_index| index <= previous_index) {
                return Err(GeometryAuthoringSemanticError::SparseIndexOrder {
                    attribute: attribute_index,
                    element,
                });
            }
            Ok(Some(index))
        })??;
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
        semantic_name_offset,
        semantic_name_count,
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
        GeometryAttributePayloadAuthoring::DenseF64 { lanes, values } => scalar_element_count(
            attribute_index,
            lanes,
            values.len(),
            "f64 attribute elements",
        ),
        GeometryAttributePayloadAuthoring::DenseU32 { lanes, values } => scalar_element_count(
            attribute_index,
            lanes,
            values.len(),
            "u32 attribute elements",
        ),
        GeometryAttributePayloadAuthoring::DenseI32 { lanes, values } => scalar_element_count(
            attribute_index,
            lanes,
            values.len(),
            "i32 attribute elements",
        ),
        GeometryAttributePayloadAuthoring::SparseF32 { logical_count, .. }
        | GeometryAttributePayloadAuthoring::SparseU32 { logical_count, .. }
        | GeometryAttributePayloadAuthoring::SparseI32 { logical_count, .. } => Ok(logical_count),
    }
}

fn validate_attribute_authored_shape(
    attribute_index: usize,
    attribute: &GeometryAttributeAuthoring<'_>,
) -> Result<(), GeometryAuthoringSemanticError> {
    match attribute.payload() {
        GeometryAttributePayloadAuthoring::DenseF32 {
            lanes,
            source_values,
            source_indices,
        } => {
            scalar_element_count(
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
            if let Presence::Present(indices) = source_indices {
                if attribute.domain() != AttributeDomain::Corner {
                    return Err(GeometryAuthoringSemanticError::SourceProjectionDomain {
                        attribute: attribute_index,
                    });
                }
                collection_count(indices.len(), "attribute source projection elements")?;
            }
        }
        GeometryAttributePayloadAuthoring::DenseF64 { lanes, values } => {
            scalar_element_count(
                attribute_index,
                lanes,
                values.len(),
                "f64 attribute elements",
            )?;
            validate_attribute_schema(
                attribute_index,
                attribute,
                GeometryAttributeScalarKind::F64,
                lanes,
            )?;
        }
        GeometryAttributePayloadAuthoring::DenseU32 { lanes, values } => {
            scalar_element_count(
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
        }
        GeometryAttributePayloadAuthoring::DenseI32 { lanes, values } => {
            scalar_element_count(
                attribute_index,
                lanes,
                values.len(),
                "i32 attribute elements",
            )?;
            validate_attribute_schema(
                attribute_index,
                attribute,
                GeometryAttributeScalarKind::I32,
                lanes,
            )?;
        }
        GeometryAttributePayloadAuthoring::SparseF32 {
            lanes,
            indices,
            values,
            default,
            ..
        } => validate_sparse_attribute_authored_shape(
            attribute_index,
            attribute,
            GeometryAttributeScalarKind::F32,
            lanes,
            indices,
            values.len(),
            default.len(),
        )?,
        GeometryAttributePayloadAuthoring::SparseU32 {
            lanes,
            indices,
            values,
            default,
            ..
        } => validate_sparse_attribute_authored_shape(
            attribute_index,
            attribute,
            GeometryAttributeScalarKind::U32,
            lanes,
            indices,
            values.len(),
            default.len(),
        )?,
        GeometryAttributePayloadAuthoring::SparseI32 {
            lanes,
            indices,
            values,
            default,
            ..
        } => validate_sparse_attribute_authored_shape(
            attribute_index,
            attribute,
            GeometryAttributeScalarKind::I32,
            lanes,
            indices,
            values.len(),
            default.len(),
        )?,
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_sparse_attribute_authored_shape(
    attribute_index: usize,
    attribute: &GeometryAttributeAuthoring<'_>,
    scalar_kind: GeometryAttributeScalarKind,
    lanes: u8,
    indices: &[u32],
    value_scalar_count: usize,
    default_scalar_count: usize,
) -> Result<(), GeometryAuthoringSemanticError> {
    let explicit_element_count = scalar_element_count(
        attribute_index,
        lanes,
        value_scalar_count,
        "sparse attribute explicit elements",
    )?;
    validate_attribute_schema(attribute_index, attribute, scalar_kind, lanes)?;
    if default_scalar_count != usize::from(lanes) {
        return Err(GeometryAuthoringSemanticError::SparseDefaultShape {
            attribute: attribute_index,
            lanes,
            actual: default_scalar_count,
        });
    }
    let index_count = collection_count(indices.len(), "sparse attribute indices")?;
    if explicit_element_count != index_count {
        return Err(GeometryAuthoringSemanticError::SparseValueCount {
            attribute: attribute_index,
            index_count: indices.len(),
            value_elements: u32_to_usize(explicit_element_count, "sparse explicit element count")?,
        });
    }
    Ok(())
}

fn validate_expected_attribute_cardinality(
    attribute_index: usize,
    attribute: &GeometryAttributeAuthoring<'_>,
    expected: u32,
) -> Result<(), GeometryAuthoringSemanticError> {
    if let GeometryAttributePayloadAuthoring::DenseF32 {
        source_indices: Presence::Present(indices),
        ..
    } = attribute.payload()
    {
        let actual = collection_count(indices.len(), "attribute source projection elements")?;
        if actual != expected {
            return Err(
                GeometryAuthoringSemanticError::SourceProjectionCardinality {
                    attribute: attribute_index,
                    expected,
                    actual: indices.len(),
                },
            );
        }
        return Ok(());
    }

    check_attribute_cardinality(
        attribute_index,
        attribute.domain(),
        attribute_element_count(attribute_index, attribute)?,
        Some(expected),
    )
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
        AttributeSemanticKind::Custom => {
            matches!(
                scalar_kind,
                GeometryAttributeScalarKind::F32
                    | GeometryAttributeScalarKind::F64
                    | GeometryAttributeScalarKind::U32
                    | GeometryAttributeScalarKind::I32
            ) && (1..=16).contains(&lanes)
        }
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
    C: CanonicalAuthoringControl,
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
    C: CanonicalAuthoringControl,
{
    for (relative_scalar, value) in values.iter().enumerate() {
        context.run_unit(SceneValidationWork::PayloadCell, || {
            if !value.is_finite() {
                let scalar = scalar_offset.checked_add(relative_scalar).ok_or(
                    GeometryAuthoringSemanticError::AccountingOverflow {
                        dimension: "attribute scalar index",
                    },
                )?;
                return Err(GeometryAuthoringSemanticError::NonFiniteAttributeValue {
                    attribute: attribute_index,
                    scalar,
                });
            }
            Ok(())
        })??;
    }
    Ok(())
}

fn inspect_f64_values<C>(
    attribute_index: usize,
    values: &[f64],
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    for (scalar, value) in values.iter().enumerate() {
        context.run_unit(SceneValidationWork::PayloadCell, || {
            if value.is_finite() {
                Ok(())
            } else {
                Err(GeometryAuthoringSemanticError::NonFiniteAttributeValue {
                    attribute: attribute_index,
                    scalar,
                })
            }
        })??;
    }
    Ok(())
}

fn inspect_u32_values<C>(
    values: &[u32],
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    for _ in values {
        context.run_unit(SceneValidationWork::PayloadCell, || ())?;
    }
    Ok(())
}

fn inspect_i32_values<C>(
    values: &[i32],
    context: &mut GeometryValidationContext<'_, C>,
) -> GeometryValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    for _ in values {
        context.run_unit(SceneValidationWork::PayloadCell, || ())?;
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

fn scratch_envelope(
    plan: CapacityPlan,
    resource: &'static str,
) -> Result<u64, GeometryAuthoringSemanticError> {
    plan.envelope_delta_bytes()
        .map_err(|cause| GeometryAuthoringSemanticError::ScratchCapacity { resource, cause })
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
