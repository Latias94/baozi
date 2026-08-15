#![allow(
    clippy::result_large_err,
    reason = "deformation authoring failures retain bounded control evidence inline without a failure-path allocation"
)]

use std::error::Error;
use std::fmt;

use baozi_operation::{
    CapacityPlan, CapacityPlanError, OperationOwner, ProgressPhase, ProgressUnit,
    RetainedStoreReservation,
};
use thiserror::Error;

use crate::deform::{MorphDeltaSemantic, SkinMatrixSpace, SkinMatrixSpaces};
use crate::{SCENE_VALIDATION_POLL_INTERVAL, SceneValidationLimits, SceneValidationWork};

use super::super::super::schema::deformation::{
    DEFORMATION_DIRECTORY_FIXED_BYTES, DEFORMATION_LOGICAL_MORPH_FIXED_BYTES,
    DEFORMATION_MESH_INSTANCE_FIXED_BYTES, DEFORMATION_MORPH_DEFINITION_FIXED_BYTES,
    DEFORMATION_MORPH_DELTA_FIXED_BYTES, DEFORMATION_MORPH_TARGET_FIXED_BYTES,
    DEFORMATION_PRIMITIVE_FIXED_BYTES, DEFORMATION_SKIN_FIXED_BYTES,
};
use super::super::control::{
    CanonicalAuthoringControl, OperationAuthoringControl, OperationAuthoringControlError,
};
use super::super::input::Presence;
use super::super::input::deformation::{
    DeformationAuthoringBatch, InfluenceRowsAuthoring, LogicalMorphAuthoring,
    MeshInstanceDeformationAuthoring, MorphDeltaAuthoring, MorphDeltaPayloadAuthoring,
    MorphTargetDefinitionAuthoring, PrimitiveDeformationAuthoring, PrimitiveMorphTargetAuthoring,
    SkinAuthoring, SkinLocalId,
};
use super::super::input::geometry::AuthoringRange;
use super::ValidatedNodeBatch;
use super::geometry::ValidatedGeometryBatch;

const LOGICAL_U8_BYTES: u64 = 1;
const LOGICAL_U32_BYTES: u64 = 4;
const LOGICAL_F32_BYTES: u64 = 4;
const LOGICAL_REFERENCE_BYTES: u64 = 4;
const MORPH_DELTA_LANES: u32 = 3;
const INVERSE_BIND_SCALARS: u32 = 16;
const INVERSE_BIND_SCALAR_COUNT: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum DeformationAuthoringLimit {
    Entities,
    Edges,
    References,
    SemanticCells,
    CanonicalBytes,
    Work,
    ScratchBytes,
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum DeformationAuthoringSemanticError {
    #[error("deformation authoring count for {collection} exceeds the u32 canonical domain")]
    CountDomainTooLarge { collection: &'static str },
    #[error("{limit:?} value {actual} exceeds limit {maximum}")]
    LimitExceeded {
        limit: DeformationAuthoringLimit,
        actual: u64,
        maximum: u64,
    },
    #[error("deformation authoring accounting overflowed for {dimension}")]
    AccountingOverflow { dimension: &'static str },
    #[error("deformation table {table} has {actual} rows; validated geometry requires {expected}")]
    GeometryTableCardinality {
        table: &'static str,
        expected: u32,
        actual: usize,
    },
    #[error("skin {skin} uses a non-canonical matrix-space contract")]
    NonCanonicalSkinMatrixSpaces { skin: usize },
    #[error("skin {skin} has {joint_count} joints but {inverse_bind_count} inverse-bind matrices")]
    InverseBindCountMismatch {
        skin: usize,
        joint_count: usize,
        inverse_bind_count: usize,
    },
    #[error(
        "skin {skin} joint {joint} references node {node} beyond authored node count {node_count}"
    )]
    JointNodeOutOfRange {
        skin: usize,
        joint: usize,
        node: u32,
        node_count: usize,
    },
    #[error(
        "skin {skin} skeleton root references node {node} beyond authored node count {node_count}"
    )]
    SkeletonRootOutOfRange {
        skin: usize,
        node: u32,
        node_count: usize,
    },
    #[error("skin {skin} inverse-bind matrix {joint} contains a non-finite scalar")]
    NonFiniteInverseBind { skin: usize, joint: usize },
    #[error("skin {skin} inverse-bind matrix {joint} is not exactly affine")]
    NonAffineInverseBind { skin: usize, joint: usize },
    #[error("skin {skin} correction matrix contains a non-finite scalar")]
    NonFiniteSkinCorrection { skin: usize },
    #[error("skin {skin} correction matrix is not exactly affine")]
    NonAffineSkinCorrection { skin: usize },
    #[error(
        "skin {skin} correction maps {from:?} to {to:?}; canonical composition requires {expected_from:?} to {expected_to:?}"
    )]
    ContradictorySkinCorrectionSpaces {
        skin: usize,
        from: SkinMatrixSpace,
        to: SkinMatrixSpace,
        expected_from: SkinMatrixSpace,
        expected_to: SkinMatrixSpace,
    },
    #[error(
        "logical mesh {logical_mesh} definition range starts at {actual}; expected contiguous offset {expected}"
    )]
    MorphDefinitionRangeOrder {
        logical_mesh: usize,
        expected: u32,
        actual: u32,
    },
    #[error(
        "logical mesh {logical_mesh} definition range [{offset}, {end}) exceeds definition count {definition_count}"
    )]
    MorphDefinitionRangeOutOfRange {
        logical_mesh: usize,
        offset: u32,
        end: u64,
        definition_count: u32,
    },
    #[error("logical morph ranges own {owned} definitions; the batch contains {definition_count}")]
    UnownedMorphDefinitions { owned: u32, definition_count: u32 },
    #[error(
        "logical mesh {logical_mesh} owns {definition_count} morph definitions but has {default_count} default weights"
    )]
    LogicalMorphWidthMismatch {
        logical_mesh: usize,
        definition_count: u32,
        default_count: usize,
    },
    #[error(
        "logical mesh {logical_mesh} repeats morph target identity {identity} at target {target}"
    )]
    DuplicateMorphTargetIdentity {
        logical_mesh: usize,
        target: usize,
        identity: u32,
    },
    #[error(
        "logical mesh {logical_mesh} morph target {target} identity {identity} exceeds target width {target_count}"
    )]
    MorphTargetIdentityOutOfRange {
        logical_mesh: usize,
        target: usize,
        identity: u32,
        target_count: u32,
    },
    #[error("logical mesh {logical_mesh} default morph weight {target} is non-finite")]
    NonFiniteDefaultMorphWeight { logical_mesh: usize, target: usize },
    #[error(
        "primitive {primitive} exposes {actual} morph targets; logical mesh requires {expected}"
    )]
    PrimitiveMorphTargetCountMismatch {
        primitive: usize,
        expected: u32,
        actual: usize,
    },
    #[error(
        "primitive {primitive} influence offsets contain {actual} entries; vertex count requires {expected}"
    )]
    InfluenceOffsetCountMismatch {
        primitive: usize,
        expected: u64,
        actual: usize,
    },
    #[error("primitive {primitive} first influence offset is {actual}; expected zero")]
    InfluenceFirstOffset { primitive: usize, actual: u32 },
    #[error(
        "primitive {primitive} influence offset {offset} at position {position} exceeds influence count {influence_count}"
    )]
    InfluenceOffsetOutOfRange {
        primitive: usize,
        position: usize,
        offset: u32,
        influence_count: u32,
    },
    #[error(
        "primitive {primitive} influence offsets decrease at position {position}: {previous} to {current}"
    )]
    InfluenceOffsetOrder {
        primitive: usize,
        position: usize,
        previous: u32,
        current: u32,
    },
    #[error("primitive {primitive} final influence offset is {actual}; expected {expected}")]
    InfluenceFinalOffset {
        primitive: usize,
        expected: u32,
        actual: u32,
    },
    #[error("primitive {primitive} influence {influence} has a non-finite weight")]
    NonFiniteInfluenceWeight { primitive: usize, influence: usize },
    #[error("primitive {primitive} influence {influence} has a negative weight")]
    NegativeInfluenceWeight { primitive: usize, influence: usize },
    #[error("primitive {primitive} vertex {vertex} repeats source order {source_order}")]
    DuplicateInfluenceSourceOrder {
        primitive: usize,
        vertex: u32,
        source_order: u32,
    },
    #[error(
        "primitive {primitive} vertex {vertex} influence {influence} has source order {source_order}; row width is {row_width}"
    )]
    InfluenceSourceOrderOutOfRange {
        primitive: usize,
        vertex: u32,
        influence: usize,
        source_order: u32,
        row_width: u32,
    },
    #[error("primitive {primitive} morph target {target} repeats {semantic:?} at delta {delta}")]
    DuplicateMorphSemantic {
        primitive: usize,
        target: usize,
        delta: usize,
        semantic: MorphDeltaSemantic,
    },
    #[error(
        "primitive {primitive} morph target {target} delta {delta} is not in Position, Normal, Tangent order"
    )]
    MorphSemanticOrder {
        primitive: usize,
        target: usize,
        delta: usize,
    },
    #[error(
        "primitive {primitive} morph target {target} {semantic:?} dense delta has {actual} scalars; expected {expected}"
    )]
    DenseMorphScalarCount {
        primitive: usize,
        target: usize,
        semantic: MorphDeltaSemantic,
        expected: u64,
        actual: usize,
    },
    #[error(
        "primitive {primitive} morph target {target} {semantic:?} sparse logical count {actual} differs from vertex count {expected}"
    )]
    SparseMorphLogicalCount {
        primitive: usize,
        target: usize,
        semantic: MorphDeltaSemantic,
        expected: u32,
        actual: u32,
    },
    #[error(
        "primitive {primitive} morph target {target} {semantic:?} sparse delta has {actual} scalars for {index_count} indices"
    )]
    SparseMorphScalarCount {
        primitive: usize,
        target: usize,
        semantic: MorphDeltaSemantic,
        index_count: usize,
        actual: usize,
    },
    #[error(
        "primitive {primitive} morph target {target} {semantic:?} sparse index {index} at element {element} exceeds logical count {logical_count}"
    )]
    SparseMorphIndexOutOfRange {
        primitive: usize,
        target: usize,
        semantic: MorphDeltaSemantic,
        element: usize,
        index: u32,
        logical_count: u32,
    },
    #[error(
        "primitive {primitive} morph target {target} {semantic:?} sparse indices are not strictly increasing at element {element}"
    )]
    SparseMorphIndexOrder {
        primitive: usize,
        target: usize,
        semantic: MorphDeltaSemantic,
        element: usize,
    },
    #[error(
        "primitive {primitive} morph target {target} {semantic:?} delta scalar {scalar} is non-finite"
    )]
    NonFiniteMorphDelta {
        primitive: usize,
        target: usize,
        semantic: MorphDeltaSemantic,
        scalar: usize,
    },
    #[error("mesh instance {instance} references skin {skin} beyond skin count {skin_count}")]
    InstanceSkinOutOfRange {
        instance: usize,
        skin: u32,
        skin_count: u32,
    },
    #[error(
        "mesh instance {instance} morph override has width {actual}; logical mesh requires {expected}"
    )]
    InstanceMorphWidth {
        instance: usize,
        expected: u32,
        actual: usize,
    },
    #[error("mesh instance {instance} morph override weight {target} is non-finite")]
    NonFiniteInstanceMorphWeight { instance: usize, target: usize },
    #[error(
        "mesh instance {instance} selects skin {skin} with {joint_count} joints, but logical mesh {logical_mesh} primitive {primitive} influence {influence} uses palette joint {palette_joint}"
    )]
    PaletteJointOutOfRange {
        instance: usize,
        logical_mesh: u32,
        skin: u32,
        joint_count: u32,
        primitive: u32,
        influence: u32,
        palette_joint: u32,
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
pub(in crate::canonical_scene::store) enum DeformationAuthoringFailure<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Semantic(#[from] DeformationAuthoringSemanticError),
    #[error("deformation authoring validation control failed")]
    Control(#[source] E),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum DeformationMorphPayloadKind {
    Dense,
    Sparse,
}

#[derive(Debug, Clone, Copy)]
struct PaletteJointLocation {
    primitive: u32,
    influence: u32,
    palette_joint: u32,
}

#[derive(Debug, Clone, Copy)]
struct ValidatedSkinPlan {
    name_byte_offset: u32,
    name_byte_count: u32,
    joint_offset: u32,
    joint_count: u32,
    inverse_bind_scalar_offset: u32,
    correction_matrix_scalar_offset: u32,
    correction_matrix_scalar_count: u32,
}

#[derive(Debug, Clone, Copy)]
struct ValidatedLogicalMorphPlan {
    definitions: AuthoringRange,
    default_weight_offset: u32,
    target_count: u32,
    maximum_palette_joint: Option<PaletteJointLocation>,
}

#[derive(Debug, Clone, Copy)]
struct ValidatedMorphTargetDefinitionPlan {
    logical_mesh: u32,
    target_ordinal: u32,
    name_byte_offset: u32,
    name_byte_count: u32,
}

#[derive(Debug, Clone, Copy)]
struct ValidatedPrimitiveDeformationPlan {
    influence_row_offset_offset: u32,
    influence_row_offset_count: u32,
    influence_offset: u32,
    influence_count: u32,
    morph_targets: AuthoringRange,
}

#[derive(Debug, Clone, Copy)]
struct ValidatedMorphTargetPlan {
    primitive_index: u32,
    target_ordinal: u32,
    deltas: AuthoringRange,
}

#[derive(Debug, Clone, Copy)]
struct ValidatedMorphDeltaPlan {
    semantic: MorphDeltaSemantic,
    payload_kind: DeformationMorphPayloadKind,
    logical_count: u32,
    explicit_count: u32,
    value_offset: u32,
    value_count: u32,
    sparse_index_offset: u32,
    sparse_index_count: u32,
}

#[derive(Debug, Clone, Copy)]
struct ValidatedMeshInstanceDeformationPlan {
    logical_mesh: u32,
    skin: Option<SkinLocalId>,
    morph_weight_offset: u32,
    morph_weight_count: u32,
}

pub(in crate::canonical_scene::store) struct ValidatedSkin<'view, 'input> {
    input: SkinAuthoring<'input>,
    plan: &'view ValidatedSkinPlan,
}

impl<'input> ValidatedSkin<'_, 'input> {
    pub(in crate::canonical_scene::store) const fn authored(&self) -> SkinAuthoring<'input> {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn name_byte_offset(&self) -> u32 {
        self.plan.name_byte_offset
    }

    pub(in crate::canonical_scene::store) const fn name_byte_count(&self) -> u32 {
        self.plan.name_byte_count
    }

    pub(in crate::canonical_scene::store) const fn joint_offset(&self) -> u32 {
        self.plan.joint_offset
    }

    pub(in crate::canonical_scene::store) const fn joint_count(&self) -> u32 {
        self.plan.joint_count
    }

    pub(in crate::canonical_scene::store) const fn inverse_bind_scalar_offset(&self) -> u32 {
        self.plan.inverse_bind_scalar_offset
    }

    pub(in crate::canonical_scene::store) const fn correction_matrix_scalar_offset(&self) -> u32 {
        self.plan.correction_matrix_scalar_offset
    }

    pub(in crate::canonical_scene::store) const fn correction_matrix_scalar_count(&self) -> u32 {
        self.plan.correction_matrix_scalar_count
    }
}

pub(in crate::canonical_scene::store) struct ValidatedLogicalMorph<'view, 'input> {
    input: LogicalMorphAuthoring<'input>,
    plan: &'view ValidatedLogicalMorphPlan,
}

impl<'input> ValidatedLogicalMorph<'_, 'input> {
    pub(in crate::canonical_scene::store) const fn authored(
        &self,
    ) -> LogicalMorphAuthoring<'input> {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn default_weight_offset(&self) -> u32 {
        self.plan.default_weight_offset
    }

    pub(in crate::canonical_scene::store) const fn definitions(&self) -> AuthoringRange {
        self.plan.definitions
    }

    pub(in crate::canonical_scene::store) const fn target_count(&self) -> u32 {
        self.plan.target_count
    }
}

pub(in crate::canonical_scene::store) struct ValidatedMorphTargetDefinition<'view, 'input> {
    input: MorphTargetDefinitionAuthoring<'input>,
    plan: &'view ValidatedMorphTargetDefinitionPlan,
}

impl<'input> ValidatedMorphTargetDefinition<'_, 'input> {
    pub(in crate::canonical_scene::store) const fn authored(
        &self,
    ) -> MorphTargetDefinitionAuthoring<'input> {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn logical_mesh(&self) -> u32 {
        self.plan.logical_mesh
    }

    pub(in crate::canonical_scene::store) const fn target_ordinal(&self) -> u32 {
        self.plan.target_ordinal
    }

    pub(in crate::canonical_scene::store) const fn name_byte_offset(&self) -> u32 {
        self.plan.name_byte_offset
    }

    pub(in crate::canonical_scene::store) const fn name_byte_count(&self) -> u32 {
        self.plan.name_byte_count
    }
}

pub(in crate::canonical_scene::store) struct ValidatedPrimitiveDeformation<'view, 'input> {
    input: PrimitiveDeformationAuthoring<'input>,
    plan: &'view ValidatedPrimitiveDeformationPlan,
    targets: &'view [ValidatedMorphTargetPlan],
    deltas: &'view [ValidatedMorphDeltaPlan],
}

impl<'view, 'input> ValidatedPrimitiveDeformation<'view, 'input> {
    pub(in crate::canonical_scene::store) const fn authored(
        &self,
    ) -> PrimitiveDeformationAuthoring<'input> {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn influence_row_offset_offset(&self) -> u32 {
        self.plan.influence_row_offset_offset
    }

    pub(in crate::canonical_scene::store) const fn influence_row_offset_count(&self) -> u32 {
        self.plan.influence_row_offset_count
    }

    pub(in crate::canonical_scene::store) const fn influence_offset(&self) -> u32 {
        self.plan.influence_offset
    }

    pub(in crate::canonical_scene::store) const fn influence_count(&self) -> u32 {
        self.plan.influence_count
    }

    pub(in crate::canonical_scene::store) const fn morph_target_count(&self) -> usize {
        self.targets.len()
    }

    pub(in crate::canonical_scene::store) fn morph_target(
        &self,
        index: usize,
    ) -> Option<ValidatedMorphTarget<'_, 'input>> {
        let input = *self.input.morph_targets().get(index)?;
        let plan = self.targets.get(index)?;
        let delta_range = checked_range(plan.deltas, self.deltas.len()).ok()?;
        Some(ValidatedMorphTarget {
            input,
            plan,
            deltas: self.deltas.get(delta_range)?,
        })
    }
}

pub(in crate::canonical_scene::store) struct ValidatedMorphTarget<'view, 'input> {
    input: PrimitiveMorphTargetAuthoring<'input>,
    plan: &'view ValidatedMorphTargetPlan,
    deltas: &'view [ValidatedMorphDeltaPlan],
}

impl<'view, 'input> ValidatedMorphTarget<'view, 'input> {
    pub(in crate::canonical_scene::store) const fn primitive_index(&self) -> u32 {
        self.plan.primitive_index
    }

    pub(in crate::canonical_scene::store) const fn target_ordinal(&self) -> u32 {
        self.plan.target_ordinal
    }

    pub(in crate::canonical_scene::store) const fn delta_count(&self) -> usize {
        self.deltas.len()
    }

    pub(in crate::canonical_scene::store) fn delta(
        &self,
        index: usize,
    ) -> Option<ValidatedMorphDelta<'_, 'input>> {
        Some(ValidatedMorphDelta {
            input: *self.input.deltas().get(index)?,
            plan: self.deltas.get(index)?,
        })
    }
}

pub(in crate::canonical_scene::store) struct ValidatedMorphDelta<'view, 'input> {
    input: MorphDeltaAuthoring<'input>,
    plan: &'view ValidatedMorphDeltaPlan,
}

impl<'input> ValidatedMorphDelta<'_, 'input> {
    pub(in crate::canonical_scene::store) const fn authored(&self) -> MorphDeltaAuthoring<'input> {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn authored_payload(
        &self,
    ) -> MorphDeltaPayloadAuthoring<'input> {
        self.input.payload()
    }

    pub(in crate::canonical_scene::store) const fn semantic(&self) -> MorphDeltaSemantic {
        self.plan.semantic
    }

    pub(in crate::canonical_scene::store) const fn payload_kind(
        &self,
    ) -> DeformationMorphPayloadKind {
        self.plan.payload_kind
    }

    pub(in crate::canonical_scene::store) const fn logical_count(&self) -> u32 {
        self.plan.logical_count
    }

    pub(in crate::canonical_scene::store) const fn explicit_count(&self) -> u32 {
        self.plan.explicit_count
    }

    pub(in crate::canonical_scene::store) const fn value_offset(&self) -> u32 {
        self.plan.value_offset
    }

    pub(in crate::canonical_scene::store) const fn value_count(&self) -> u32 {
        self.plan.value_count
    }

    pub(in crate::canonical_scene::store) const fn sparse_index_offset(&self) -> u32 {
        self.plan.sparse_index_offset
    }

    pub(in crate::canonical_scene::store) const fn sparse_index_count(&self) -> u32 {
        self.plan.sparse_index_count
    }
}

pub(in crate::canonical_scene::store) struct ValidatedMeshInstanceDeformation<'view, 'input> {
    input: MeshInstanceDeformationAuthoring<'input>,
    plan: &'view ValidatedMeshInstanceDeformationPlan,
}

impl<'input> ValidatedMeshInstanceDeformation<'_, 'input> {
    pub(in crate::canonical_scene::store) const fn authored(
        &self,
    ) -> MeshInstanceDeformationAuthoring<'input> {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn logical_mesh(&self) -> u32 {
        self.plan.logical_mesh
    }

    pub(in crate::canonical_scene::store) const fn skin(&self) -> Option<SkinLocalId> {
        self.plan.skin
    }

    pub(in crate::canonical_scene::store) const fn morph_weight_offset(&self) -> u32 {
        self.plan.morph_weight_offset
    }

    pub(in crate::canonical_scene::store) const fn morph_weight_count(&self) -> u32 {
        self.plan.morph_weight_count
    }
}

#[must_use = "dropping the validated deformation batch releases its compiled-plan reservation"]
pub(in crate::canonical_scene::store) struct ValidatedDeformationBatch<'input, R> {
    input: DeformationAuthoringBatch<'input>,
    skin_count: u32,
    logical_morph_count: u32,
    morph_definition_count: u32,
    primitive_count: u32,
    morph_target_count: u32,
    morph_delta_count: u32,
    mesh_instance_count: u32,
    joint_node_count: u32,
    inverse_bind_scalar_count: u32,
    skin_correction_scalar_count: u32,
    skin_name_byte_count: u32,
    influence_row_offset_count: u32,
    influence_count: u32,
    morph_default_weight_count: u32,
    morph_name_byte_count: u32,
    morph_value_count: u32,
    morph_sparse_index_count: u32,
    instance_morph_weight_count: u32,
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
    maximum_uniqueness_capacity: usize,
    skin_plans: Vec<ValidatedSkinPlan>,
    logical_morph_plans: Vec<ValidatedLogicalMorphPlan>,
    morph_definition_plans: Vec<ValidatedMorphTargetDefinitionPlan>,
    primitive_plans: Vec<ValidatedPrimitiveDeformationPlan>,
    morph_target_plans: Vec<ValidatedMorphTargetPlan>,
    morph_delta_plans: Vec<ValidatedMorphDeltaPlan>,
    mesh_instance_plans: Vec<ValidatedMeshInstanceDeformationPlan>,
    // Every compiled-plan allocation above must drop before its accounting token.
    reservation: R,
}

impl<'input, R> ValidatedDeformationBatch<'input, R> {
    pub(in crate::canonical_scene::store) const fn input(
        &self,
    ) -> DeformationAuthoringBatch<'input> {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn skin_count(&self) -> u32 {
        self.skin_count
    }

    pub(in crate::canonical_scene::store) const fn logical_morph_count(&self) -> u32 {
        self.logical_morph_count
    }

    pub(in crate::canonical_scene::store) const fn morph_definition_count(&self) -> u32 {
        self.morph_definition_count
    }

    pub(in crate::canonical_scene::store) const fn primitive_count(&self) -> u32 {
        self.primitive_count
    }

    pub(in crate::canonical_scene::store) const fn morph_target_count(&self) -> u32 {
        self.morph_target_count
    }

    pub(in crate::canonical_scene::store) const fn morph_delta_count(&self) -> u32 {
        self.morph_delta_count
    }

    pub(in crate::canonical_scene::store) const fn mesh_instance_count(&self) -> u32 {
        self.mesh_instance_count
    }

    pub(in crate::canonical_scene::store) const fn joint_node_count(&self) -> u32 {
        self.joint_node_count
    }

    pub(in crate::canonical_scene::store) const fn inverse_bind_scalar_count(&self) -> u32 {
        self.inverse_bind_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn skin_correction_scalar_count(&self) -> u32 {
        self.skin_correction_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn skin_name_byte_count(&self) -> u32 {
        self.skin_name_byte_count
    }

    pub(in crate::canonical_scene::store) const fn influence_row_offset_count(&self) -> u32 {
        self.influence_row_offset_count
    }

    pub(in crate::canonical_scene::store) const fn influence_count(&self) -> u32 {
        self.influence_count
    }

    pub(in crate::canonical_scene::store) const fn morph_default_weight_count(&self) -> u32 {
        self.morph_default_weight_count
    }

    pub(in crate::canonical_scene::store) const fn morph_name_byte_count(&self) -> u32 {
        self.morph_name_byte_count
    }

    pub(in crate::canonical_scene::store) const fn morph_value_count(&self) -> u32 {
        self.morph_value_count
    }

    pub(in crate::canonical_scene::store) const fn morph_sparse_index_count(&self) -> u32 {
        self.morph_sparse_index_count
    }

    pub(in crate::canonical_scene::store) const fn instance_morph_weight_count(&self) -> u32 {
        self.instance_morph_weight_count
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

    pub(in crate::canonical_scene::store) const fn maximum_uniqueness_capacity(&self) -> usize {
        self.maximum_uniqueness_capacity
    }

    pub(in crate::canonical_scene::store) fn skin(
        &self,
        index: usize,
    ) -> Option<ValidatedSkin<'_, 'input>> {
        Some(ValidatedSkin {
            input: *self.input.skins().get(index)?,
            plan: self.skin_plans.get(index)?,
        })
    }

    pub(in crate::canonical_scene::store) fn logical_morph(
        &self,
        index: usize,
    ) -> Option<ValidatedLogicalMorph<'_, 'input>> {
        Some(ValidatedLogicalMorph {
            input: *self.input.logical_meshes().get(index)?,
            plan: self.logical_morph_plans.get(index)?,
        })
    }

    pub(in crate::canonical_scene::store) fn morph_definition(
        &self,
        index: usize,
    ) -> Option<ValidatedMorphTargetDefinition<'_, 'input>> {
        Some(ValidatedMorphTargetDefinition {
            input: *self.input.morph_target_definitions().get(index)?,
            plan: self.morph_definition_plans.get(index)?,
        })
    }

    pub(in crate::canonical_scene::store) fn morph_definitions(
        &self,
    ) -> impl ExactSizeIterator<Item = ValidatedMorphTargetDefinition<'_, 'input>> + '_ {
        self.input
            .morph_target_definitions()
            .iter()
            .copied()
            .zip(&self.morph_definition_plans)
            .map(|(input, plan)| ValidatedMorphTargetDefinition { input, plan })
    }

    pub(in crate::canonical_scene::store) fn primitive(
        &self,
        index: usize,
    ) -> Option<ValidatedPrimitiveDeformation<'_, 'input>> {
        let plan = self.primitive_plans.get(index)?;
        let target_range = checked_range(plan.morph_targets, self.morph_target_plans.len()).ok()?;
        Some(ValidatedPrimitiveDeformation {
            input: *self.input.primitives().get(index)?,
            plan,
            targets: self.morph_target_plans.get(target_range)?,
            deltas: &self.morph_delta_plans,
        })
    }

    pub(in crate::canonical_scene::store) fn mesh_instance(
        &self,
        index: usize,
    ) -> Option<ValidatedMeshInstanceDeformation<'_, 'input>> {
        Some(ValidatedMeshInstanceDeformation {
            input: *self.input.mesh_instances().get(index)?,
            plan: self.mesh_instance_plans.get(index)?,
        })
    }
}

impl<R> fmt::Debug for ValidatedDeformationBatch<'_, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedDeformationBatch")
            .field("skin_count", &self.skin_count)
            .field("logical_morph_count", &self.logical_morph_count)
            .field("morph_definition_count", &self.morph_definition_count)
            .field("primitive_count", &self.primitive_count)
            .field("morph_target_count", &self.morph_target_count)
            .field("morph_delta_count", &self.morph_delta_count)
            .field("mesh_instance_count", &self.mesh_instance_count)
            .field("influence_count", &self.influence_count)
            .field("canonical_bytes", &self.canonical_bytes)
            .field("validation_work", &self.validation_work)
            .field("scratch_bytes", &self.scratch_bytes)
            .field(
                "maximum_uniqueness_capacity",
                &self.maximum_uniqueness_capacity,
            )
            .field("reservation", &"<held>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
struct DeformationScratchShape {
    morph_definition_count: usize,
    morph_target_count: usize,
    morph_delta_count: usize,
    maximum_uniqueness_count: usize,
    joint_node_count: u64,
    inverse_bind_scalar_count: u64,
    skin_correction_scalar_count: u64,
    skin_name_byte_count: u64,
    influence_row_offset_count: u64,
    influence_count: u64,
    morph_default_weight_count: u64,
    morph_name_byte_count: u64,
    morph_value_count: u64,
    morph_sparse_index_count: u64,
    instance_morph_weight_count: u64,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct DeformationShapeScratchEnvelope {
    peak_bytes: u64,
    retained_bytes: u64,
}

impl DeformationShapeScratchEnvelope {
    pub(in crate::canonical_scene::store) const fn peak_bytes(self) -> u64 {
        self.peak_bytes
    }

    pub(in crate::canonical_scene::store) const fn retained_bytes(self) -> u64 {
        self.retained_bytes
    }
}

pub(in crate::canonical_scene::store) fn deformation_shape_scratch_envelope(
    input: DeformationAuthoringBatch<'_>,
) -> Result<DeformationShapeScratchEnvelope, DeformationAuthoringSemanticError> {
    let shape = deformation_scratch_shape(input)?;
    let plans = deformation_capacity_plans(input, shape)?;
    deformation_shape_scratch_envelope_from_plans(plans)
}

fn deformation_shape_scratch_envelope_from_plans(
    plans: DeformationCapacityPlans,
) -> Result<DeformationShapeScratchEnvelope, DeformationAuthoringSemanticError> {
    let retained_bytes = checked_sum_bytes(
        [
            scratch_envelope(plans.skins, "validated skin plans")?,
            scratch_envelope(plans.logical_morphs, "validated logical morph plans")?,
            scratch_envelope(plans.morph_definitions, "validated morph definition plans")?,
            scratch_envelope(plans.primitives, "validated primitive deformation plans")?,
            scratch_envelope(plans.morph_targets, "validated morph target plans")?,
            scratch_envelope(plans.morph_deltas, "validated morph delta plans")?,
            scratch_envelope(plans.mesh_instances, "validated mesh instance plans")?,
        ],
        "retained deformation shape scratch envelope",
    )?;
    let peak_bytes = retained_bytes
        .checked_add(scratch_envelope(
            plans.uniqueness,
            "deformation uniqueness scratch",
        )?)
        .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
            dimension: "deformation shape scratch envelope",
        })?;
    Ok(DeformationShapeScratchEnvelope {
        peak_bytes,
        retained_bytes,
    })
}

#[derive(Clone, Copy)]
struct DeformationCapacityPlans {
    skins: CapacityPlan,
    logical_morphs: CapacityPlan,
    morph_definitions: CapacityPlan,
    primitives: CapacityPlan,
    morph_targets: CapacityPlan,
    morph_deltas: CapacityPlan,
    mesh_instances: CapacityPlan,
    uniqueness: CapacityPlan,
}

struct DeformationValidationStorage<R> {
    skin_plans: Vec<ValidatedSkinPlan>,
    logical_morph_plans: Vec<ValidatedLogicalMorphPlan>,
    morph_definition_plans: Vec<ValidatedMorphTargetDefinitionPlan>,
    primitive_plans: Vec<ValidatedPrimitiveDeformationPlan>,
    morph_target_plans: Vec<ValidatedMorphTargetPlan>,
    morph_delta_plans: Vec<ValidatedMorphDeltaPlan>,
    mesh_instance_plans: Vec<ValidatedMeshInstanceDeformationPlan>,
    uniqueness: Vec<u8>,
    // Every allocation above must drop before its accounting token.
    reservation: R,
}

#[derive(Default)]
struct SkinValidationCursors {
    joint_nodes: u32,
    inverse_bind_scalars: u32,
    correction_scalars: u32,
    name_bytes: u32,
}

type DeformationValidationResult<T, C> =
    Result<T, DeformationAuthoringFailure<<C as CanonicalAuthoringControl>::Error>>;

struct DeformationValidationContext<'control, C: CanonicalAuthoringControl> {
    control: &'control mut C,
    limits: SceneValidationLimits,
    work: u64,
    work_since_poll: u64,
    edges: u64,
    references: u64,
    semantic_cells: u64,
    f32_scalars: u64,
    u32_scalars: u64,
    canonical_bytes: u64,
}

impl<'control, C: CanonicalAuthoringControl> DeformationValidationContext<'control, C> {
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
            canonical_bytes: 0,
        }
    }

    fn poll_now(&mut self) -> DeformationValidationResult<(), C> {
        self.control
            .poll()
            .map_err(DeformationAuthoringFailure::Control)?;
        self.work_since_poll = 0;
        Ok(())
    }

    fn finish(&mut self) -> DeformationValidationResult<(), C> {
        self.poll_now()
    }

    fn run_unit<R>(
        &mut self,
        kind: SceneValidationWork,
        action: impl FnOnce() -> R,
    ) -> DeformationValidationResult<R, C> {
        let final_work = self.work.checked_add(1).ok_or(
            DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "validation work",
            },
        )?;
        enforce_limit::<C::Error>(
            DeformationAuthoringLimit::Work,
            final_work,
            self.limits.max_work(),
        )?;

        let result = self
            .control
            .run_unit(kind, action)
            .map_err(DeformationAuthoringFailure::Control)?;
        self.work = final_work;
        self.work_since_poll += 1;
        if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
            self.poll_now()?;
        }
        Ok(result)
    }

    fn add_relations(&mut self, amount: u64) -> DeformationValidationResult<(), C> {
        for _ in 0..amount {
            let current_edges = self.edges;
            let max_edges = self.limits.max_edges();
            let final_edges = self.run_unit(SceneValidationWork::GraphEdge, || {
                let final_edges = current_edges.checked_add(1).ok_or(
                    DeformationAuthoringSemanticError::AccountingOverflow { dimension: "edges" },
                )?;
                if final_edges > max_edges {
                    return Err(DeformationAuthoringSemanticError::LimitExceeded {
                        limit: DeformationAuthoringLimit::Edges,
                        actual: final_edges,
                        maximum: max_edges,
                    });
                }
                Ok::<u64, DeformationAuthoringSemanticError>(final_edges)
            })??;
            self.edges = final_edges;

            let current_references = self.references;
            let max_references = self.limits.max_references();
            let final_references = self.run_unit(SceneValidationWork::Reference, || {
                let final_references = current_references.checked_add(1).ok_or(
                    DeformationAuthoringSemanticError::AccountingOverflow {
                        dimension: "references",
                    },
                )?;
                if final_references > max_references {
                    return Err(DeformationAuthoringSemanticError::LimitExceeded {
                        limit: DeformationAuthoringLimit::References,
                        actual: final_references,
                        maximum: max_references,
                    });
                }
                Ok::<u64, DeformationAuthoringSemanticError>(final_references)
            })??;
            self.references = final_references;
        }
        Ok(())
    }

    fn add_payload(
        &mut self,
        f32_scalars: u64,
        u32_scalars: u64,
    ) -> DeformationValidationResult<(), C> {
        let added_cells = f32_scalars.checked_add(u32_scalars).ok_or(
            DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "semantic cells",
            },
        )?;
        let semantic_cells = self.semantic_cells.checked_add(added_cells).ok_or(
            DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "semantic cells",
            },
        )?;
        let f32_total = self.f32_scalars.checked_add(f32_scalars).ok_or(
            DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "f32 scalar count",
            },
        )?;
        let u32_total = self.u32_scalars.checked_add(u32_scalars).ok_or(
            DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "u32 scalar count",
            },
        )?;
        let added_bytes = checked_sum_bytes(
            [
                checked_bytes(f32_scalars, LOGICAL_F32_BYTES, "canonical f32 payload")?,
                checked_bytes(u32_scalars, LOGICAL_U32_BYTES, "canonical u32 payload")?,
            ],
            "canonical deformation payload",
        )?;
        let canonical_bytes = self.canonical_bytes.checked_add(added_bytes).ok_or(
            DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "canonical deformation logical footprint",
            },
        )?;
        enforce_limit::<C::Error>(
            DeformationAuthoringLimit::SemanticCells,
            semantic_cells,
            self.limits.max_payload_cells(),
        )?;
        enforce_limit::<C::Error>(
            DeformationAuthoringLimit::CanonicalBytes,
            canonical_bytes,
            self.limits.max_canonical_bytes(),
        )?;
        self.semantic_cells = semantic_cells;
        self.f32_scalars = f32_total;
        self.u32_scalars = u32_total;
        self.canonical_bytes = canonical_bytes;
        Ok(())
    }

    fn add_payload_bytes(&mut self, amount: u64) -> DeformationValidationResult<(), C> {
        let semantic_cells = self.semantic_cells.checked_add(amount).ok_or(
            DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "semantic cells",
            },
        )?;
        let canonical_bytes = self.canonical_bytes.checked_add(amount).ok_or(
            DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "canonical deformation logical footprint",
            },
        )?;
        enforce_limit::<C::Error>(
            DeformationAuthoringLimit::SemanticCells,
            semantic_cells,
            self.limits.max_payload_cells(),
        )?;
        enforce_limit::<C::Error>(
            DeformationAuthoringLimit::CanonicalBytes,
            canonical_bytes,
            self.limits.max_canonical_bytes(),
        )?;
        self.semantic_cells = semantic_cells;
        self.canonical_bytes = canonical_bytes;
        Ok(())
    }

    fn set_structural_bytes(&mut self, bytes: u64) -> DeformationValidationResult<(), C> {
        enforce_limit::<C::Error>(
            DeformationAuthoringLimit::CanonicalBytes,
            bytes,
            self.limits.max_canonical_bytes(),
        )?;
        self.canonical_bytes = bytes;
        Ok(())
    }

    fn reserve_scratch(&mut self, bytes: u64) -> DeformationValidationResult<C::Reservation, C> {
        enforce_limit::<C::Error>(
            DeformationAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        let reservation = self
            .control
            .reserve_scratch(bytes)
            .map_err(DeformationAuthoringFailure::Control)?;
        self.control
            .validate_scratch(&reservation)
            .map_err(DeformationAuthoringFailure::Control)?;
        Ok(reservation)
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut C::Reservation,
        bytes: u64,
    ) -> DeformationValidationResult<(), C> {
        enforce_limit::<C::Error>(
            DeformationAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        self.control
            .resize_scratch(reservation, bytes)
            .map_err(DeformationAuthoringFailure::Control)
    }

    fn allocate_scratch<T>(
        &self,
        plan: CapacityPlan,
        resource: &'static str,
    ) -> Result<(Vec<T>, u64), DeformationAuthoringSemanticError> {
        plan.verify_preflight(0, 0).map_err(|cause| {
            DeformationAuthoringSemanticError::ScratchCapacity { resource, cause }
        })?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| DeformationAuthoringSemanticError::ScratchAllocation { resource })?;
        let actual_bytes = plan
            .finish(values.len(), values.capacity())
            .map_err(|cause| DeformationAuthoringSemanticError::ScratchCapacity {
                resource,
                cause,
            })?;
        Ok((values, actual_bytes))
    }
}

pub(in crate::canonical_scene::store) fn validate_deformation_batch<'input, NR, GR, O, PP, U>(
    input: DeformationAuthoringBatch<'input>,
    nodes: &ValidatedNodeBatch<'_, NR>,
    geometry: &ValidatedGeometryBatch<'_, GR>,
    limits: SceneValidationLimits,
    control: &mut OperationAuthoringControl<'_, O, PP, U>,
) -> Result<
    ValidatedDeformationBatch<'input, RetainedStoreReservation<O>>,
    DeformationAuthoringFailure<OperationAuthoringControlError>,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    validate_deformation_batch_inner(input, nodes, geometry, limits, control)
}

pub(in crate::canonical_scene::store) fn validate_deformation_batch_with_control<
    'input,
    NR,
    GR,
    C,
>(
    input: DeformationAuthoringBatch<'input>,
    nodes: &ValidatedNodeBatch<'_, NR>,
    geometry: &ValidatedGeometryBatch<'_, GR>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedDeformationBatch<'input, C::Reservation>, DeformationAuthoringFailure<C::Error>>
where
    C: CanonicalAuthoringControl,
{
    validate_deformation_batch_inner(input, nodes, geometry, limits, control)
}

fn validate_deformation_batch_inner<'input, NR, GR, C>(
    input: DeformationAuthoringBatch<'input>,
    nodes: &ValidatedNodeBatch<'_, NR>,
    geometry: &ValidatedGeometryBatch<'_, GR>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedDeformationBatch<'input, C::Reservation>, DeformationAuthoringFailure<C::Error>>
where
    C: CanonicalAuthoringControl,
{
    let mut context = DeformationValidationContext::new(limits, control);
    context.poll_now()?;

    require_geometry_cardinality(
        "logical morphs",
        geometry.logical_mesh_count(),
        input.logical_meshes().len(),
    )?;
    require_geometry_cardinality(
        "primitive deformation",
        geometry.primitive_count(),
        input.primitives().len(),
    )?;
    require_geometry_cardinality(
        "mesh instance deformation",
        geometry.mesh_instance_count(),
        input.mesh_instances().len(),
    )?;

    let skin_count = collection_count(input.skins().len(), "skins")?;
    let logical_morph_count = collection_count(input.logical_meshes().len(), "logical morphs")?;
    let morph_definition_count = collection_count(
        input.morph_target_definitions().len(),
        "morph target definitions",
    )?;
    let primitive_count = collection_count(input.primitives().len(), "primitive deformation")?;
    let mesh_instance_count =
        collection_count(input.mesh_instances().len(), "mesh instance deformation")?;
    let (structural_bytes, scratch_shape) = deformation_structural_bytes(input, &mut context)?;
    let morph_target_count = collection_count(scratch_shape.morph_target_count, "morph targets")?;
    let morph_delta_count = collection_count(scratch_shape.morph_delta_count, "morph deltas")?;
    let entity_count = checked_sum(
        [
            u64::from(skin_count),
            u64::from(morph_definition_count),
            u64::from(morph_target_count),
            u64::from(morph_delta_count),
        ],
        "deformation entities",
    )?;
    enforce_limit::<C::Error>(
        DeformationAuthoringLimit::Entities,
        entity_count,
        limits.max_entities(),
    )?;

    context.set_structural_bytes(structural_bytes)?;

    let capacity_plans = deformation_capacity_plans(input, scratch_shape)?;
    let scratch_envelope = deformation_shape_scratch_envelope_from_plans(capacity_plans)?;
    let reservation = context.reserve_scratch(scratch_envelope.peak_bytes())?;
    let (skin_plans, skin_bytes) =
        context.allocate_scratch(capacity_plans.skins, "validated skin plans")?;
    let (logical_morph_plans, logical_morph_bytes) = context.allocate_scratch(
        capacity_plans.logical_morphs,
        "validated logical morph plans",
    )?;
    let (morph_definition_plans, morph_definition_bytes) = context.allocate_scratch(
        capacity_plans.morph_definitions,
        "validated morph definition plans",
    )?;
    let (primitive_plans, primitive_bytes) = context.allocate_scratch(
        capacity_plans.primitives,
        "validated primitive deformation plans",
    )?;
    let (morph_target_plans, target_bytes) =
        context.allocate_scratch(capacity_plans.morph_targets, "validated morph target plans")?;
    let (morph_delta_plans, delta_bytes) =
        context.allocate_scratch(capacity_plans.morph_deltas, "validated morph delta plans")?;
    let (mesh_instance_plans, instance_bytes) = context.allocate_scratch(
        capacity_plans.mesh_instances,
        "validated mesh instance plans",
    )?;
    let (uniqueness, uniqueness_bytes) =
        context.allocate_scratch(capacity_plans.uniqueness, "deformation uniqueness scratch")?;
    let retained_scratch_bytes = checked_sum_bytes(
        [
            skin_bytes,
            logical_morph_bytes,
            morph_definition_bytes,
            primitive_bytes,
            target_bytes,
            delta_bytes,
            instance_bytes,
        ],
        "retained deformation scratch allocation",
    )?;
    let scratch_actual = retained_scratch_bytes.checked_add(uniqueness_bytes).ok_or(
        DeformationAuthoringSemanticError::AccountingOverflow {
            dimension: "deformation scratch allocation",
        },
    )?;
    let mut storage = DeformationValidationStorage {
        skin_plans,
        logical_morph_plans,
        morph_definition_plans,
        primitive_plans,
        morph_target_plans,
        morph_delta_plans,
        mesh_instance_plans,
        uniqueness,
        reservation,
    };
    context.resize_scratch(&mut storage.reservation, scratch_actual)?;

    let authored_node_count = nodes.input().nodes().len();
    let mut skin_cursors = SkinValidationCursors::default();
    validate_skins(
        input,
        authored_node_count,
        &mut skin_cursors,
        &mut storage,
        &mut context,
    )?;
    let SkinValidationCursors {
        joint_nodes: joint_node_count,
        inverse_bind_scalars: inverse_bind_scalar_count,
        correction_scalars: skin_correction_scalar_count,
        name_bytes: skin_name_byte_count,
    } = skin_cursors;

    let mut morph_default_weight_count = 0_u32;
    let mut morph_name_byte_count = 0_u32;
    validate_logical_morphs(
        input,
        &mut morph_default_weight_count,
        &mut morph_name_byte_count,
        &mut storage,
        &mut context,
    )?;

    let mut influence_row_offset_count = 0_u32;
    let mut influence_count = 0_u32;
    let mut morph_value_count = 0_u32;
    let mut morph_sparse_index_count = 0_u32;
    validate_primitives(
        input,
        geometry,
        &mut influence_row_offset_count,
        &mut influence_count,
        &mut morph_value_count,
        &mut morph_sparse_index_count,
        &mut storage,
        &mut context,
    )?;

    let mut instance_morph_weight_count = 0_u32;
    validate_mesh_instances(
        input,
        geometry,
        &mut instance_morph_weight_count,
        &mut storage,
        &mut context,
    )?;

    require_compiled_count(
        "skin joint nodes",
        joint_node_count,
        scratch_shape.joint_node_count,
    )?;
    require_compiled_count(
        "inverse-bind scalars",
        inverse_bind_scalar_count,
        scratch_shape.inverse_bind_scalar_count,
    )?;
    require_compiled_count(
        "skin correction scalars",
        skin_correction_scalar_count,
        scratch_shape.skin_correction_scalar_count,
    )?;
    require_compiled_count(
        "skin name bytes",
        skin_name_byte_count,
        scratch_shape.skin_name_byte_count,
    )?;
    require_compiled_count(
        "influence row offsets",
        influence_row_offset_count,
        scratch_shape.influence_row_offset_count,
    )?;
    require_compiled_count("influences", influence_count, scratch_shape.influence_count)?;
    require_compiled_count(
        "logical morph default weights",
        morph_default_weight_count,
        scratch_shape.morph_default_weight_count,
    )?;
    require_compiled_count(
        "morph target name bytes",
        morph_name_byte_count,
        scratch_shape.morph_name_byte_count,
    )?;
    require_compiled_count(
        "morph delta values",
        morph_value_count,
        scratch_shape.morph_value_count,
    )?;
    require_compiled_count(
        "morph sparse indices",
        morph_sparse_index_count,
        scratch_shape.morph_sparse_index_count,
    )?;
    require_compiled_count(
        "instance morph weights",
        instance_morph_weight_count,
        scratch_shape.instance_morph_weight_count,
    )?;
    require_compiled_table_count("skin plans", storage.skin_plans.len(), input.skins().len())?;
    require_compiled_table_count(
        "logical morph plans",
        storage.logical_morph_plans.len(),
        input.logical_meshes().len(),
    )?;
    require_compiled_table_count(
        "morph definition plans",
        storage.morph_definition_plans.len(),
        input.morph_target_definitions().len(),
    )?;
    require_compiled_table_count(
        "primitive deformation plans",
        storage.primitive_plans.len(),
        input.primitives().len(),
    )?;
    require_compiled_table_count(
        "morph target plans",
        storage.morph_target_plans.len(),
        scratch_shape.morph_target_count,
    )?;
    require_compiled_table_count(
        "morph delta plans",
        storage.morph_delta_plans.len(),
        scratch_shape.morph_delta_count,
    )?;
    require_compiled_table_count(
        "mesh instance deformation plans",
        storage.mesh_instance_plans.len(),
        input.mesh_instances().len(),
    )?;

    let payload_block_count = [
        joint_node_count,
        inverse_bind_scalar_count,
        skin_correction_scalar_count,
        skin_name_byte_count,
        influence_row_offset_count,
        influence_count,
        influence_count,
        influence_count,
        morph_default_weight_count,
        morph_name_byte_count,
        morph_value_count,
        morph_sparse_index_count,
        instance_morph_weight_count,
    ]
    .into_iter()
    .try_fold(0_u32, |count, cells| {
        count.checked_add(u32::from(cells != 0))
    })
    .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
        dimension: "deformation payload block count",
    })?;

    let canonical_bytes = context.canonical_bytes;
    let validation_work = context.work;
    let edge_count = context.edges;
    let reference_count = context.references;
    let semantic_cell_count = context.semantic_cells;
    let f32_scalar_count = context.f32_scalars;
    let u32_scalar_count = context.u32_scalars;
    let DeformationValidationStorage {
        skin_plans,
        logical_morph_plans,
        morph_definition_plans,
        primitive_plans,
        morph_target_plans,
        morph_delta_plans,
        mesh_instance_plans,
        uniqueness,
        reservation,
    } = storage;
    drop(uniqueness);
    let mut validated = ValidatedDeformationBatch {
        input,
        skin_count,
        logical_morph_count,
        morph_definition_count,
        primitive_count,
        morph_target_count,
        morph_delta_count,
        mesh_instance_count,
        joint_node_count,
        inverse_bind_scalar_count,
        skin_correction_scalar_count,
        skin_name_byte_count,
        influence_row_offset_count,
        influence_count,
        morph_default_weight_count,
        morph_name_byte_count,
        morph_value_count,
        morph_sparse_index_count,
        instance_morph_weight_count,
        f32_scalar_count,
        u32_scalar_count,
        payload_block_count,
        entity_count,
        edge_count,
        reference_count,
        semantic_cell_count,
        canonical_bytes,
        validation_work,
        scratch_bytes: retained_scratch_bytes,
        maximum_uniqueness_capacity: scratch_shape.maximum_uniqueness_count,
        skin_plans,
        logical_morph_plans,
        morph_definition_plans,
        primitive_plans,
        morph_target_plans,
        morph_delta_plans,
        mesh_instance_plans,
        reservation,
    };
    context.resize_scratch(&mut validated.reservation, retained_scratch_bytes)?;
    context.finish()?;
    Ok(validated)
}

fn validate_skins<C, R>(
    input: DeformationAuthoringBatch<'_>,
    authored_node_count: usize,
    cursors: &mut SkinValidationCursors,
    storage: &mut DeformationValidationStorage<R>,
    context: &mut DeformationValidationContext<'_, C>,
) -> DeformationValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    for (skin_index, skin) in input.skins().iter().copied().enumerate() {
        let matrix_spaces = skin.matrix_spaces();
        context.run_unit(SceneValidationWork::Entity, || {
            if !has_canonical_skin_base_spaces(matrix_spaces) {
                return Err(
                    DeformationAuthoringSemanticError::NonCanonicalSkinMatrixSpaces {
                        skin: skin_index,
                    },
                );
            }
            if skin.joints().len() != skin.inverse_bind_matrices().len() {
                return Err(
                    DeformationAuthoringSemanticError::InverseBindCountMismatch {
                        skin: skin_index,
                        joint_count: skin.joints().len(),
                        inverse_bind_count: skin.inverse_bind_matrices().len(),
                    },
                );
            }
            if let Some(correction) = matrix_spaces.correction()
                && (correction.from() != matrix_spaces.input_position()
                    || correction.to() != matrix_spaces.output_position())
            {
                return Err(
                    DeformationAuthoringSemanticError::ContradictorySkinCorrectionSpaces {
                        skin: skin_index,
                        from: correction.from(),
                        to: correction.to(),
                        expected_from: matrix_spaces.input_position(),
                        expected_to: matrix_spaces.output_position(),
                    },
                );
            }
            Ok::<(), DeformationAuthoringSemanticError>(())
        })??;
        let joint_count = collection_count(skin.joints().len(), "skin joints")?;
        let joint_offset =
            advance_cursor(&mut cursors.joint_nodes, joint_count, "skin joint nodes")?;
        let inverse_scalar_count = joint_count.checked_mul(INVERSE_BIND_SCALARS).ok_or(
            DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "inverse-bind scalar count",
            },
        )?;
        let inverse_bind_scalar_offset = advance_cursor(
            &mut cursors.inverse_bind_scalars,
            inverse_scalar_count,
            "inverse-bind scalars",
        )?;
        let (name_byte_offset, skin_name_byte_count) = match skin.name() {
            Presence::Absent => (0, 0),
            Presence::Present(name) => {
                let count = collection_count(name.as_str().len(), "skin name bytes")?;
                let offset = advance_cursor(&mut cursors.name_bytes, count, "skin name bytes")?;
                for byte in name.as_str().as_bytes() {
                    context.run_unit(SceneValidationWork::PayloadCell, || *byte)?;
                }
                context.add_payload_bytes(u64::from(count))?;
                (offset, count)
            }
        };

        for (joint_index, joint) in skin.joints().iter().copied().enumerate() {
            context.run_unit(SceneValidationWork::Traversal, || {
                if usize::try_from(joint.index()).map_or(true, |index| index >= authored_node_count)
                {
                    return Err(DeformationAuthoringSemanticError::JointNodeOutOfRange {
                        skin: skin_index,
                        joint: joint_index,
                        node: joint.index(),
                        node_count: authored_node_count,
                    });
                }
                Ok::<(), DeformationAuthoringSemanticError>(())
            })??;
        }
        context.add_relations(u64::from(joint_count))?;

        if let Presence::Present(root) = skin.skeleton_root() {
            context.run_unit(SceneValidationWork::Traversal, || {
                if usize::try_from(root.index()).map_or(true, |index| index >= authored_node_count)
                {
                    return Err(DeformationAuthoringSemanticError::SkeletonRootOutOfRange {
                        skin: skin_index,
                        node: root.index(),
                        node_count: authored_node_count,
                    });
                }
                Ok::<(), DeformationAuthoringSemanticError>(())
            })??;
            context.add_relations(1)?;
        }

        for (joint, matrix) in skin.inverse_bind_matrices().iter().copied().enumerate() {
            for (scalar, value) in matrix.cols.into_iter().flatten().enumerate() {
                context.run_unit(SceneValidationWork::PayloadCell, || {
                    if !value.is_finite() {
                        return Err(DeformationAuthoringSemanticError::NonFiniteInverseBind {
                            skin: skin_index,
                            joint,
                        });
                    }
                    if scalar + 1 == INVERSE_BIND_SCALAR_COUNT && !matrix.is_affine() {
                        return Err(DeformationAuthoringSemanticError::NonAffineInverseBind {
                            skin: skin_index,
                            joint,
                        });
                    }
                    Ok::<(), DeformationAuthoringSemanticError>(())
                })??;
            }
        }
        let (correction_matrix_scalar_offset, correction_matrix_scalar_count) =
            match matrix_spaces.correction() {
                None => (0, 0),
                Some(correction) => {
                    let matrix = correction.matrix();
                    for (scalar, value) in matrix.cols.into_iter().flatten().enumerate() {
                        context.run_unit(SceneValidationWork::PayloadCell, || {
                            if !value.is_finite() {
                                return Err(
                                    DeformationAuthoringSemanticError::NonFiniteSkinCorrection {
                                        skin: skin_index,
                                    },
                                );
                            }
                            if scalar + 1 == INVERSE_BIND_SCALAR_COUNT && !matrix.is_affine() {
                                return Err(
                                    DeformationAuthoringSemanticError::NonAffineSkinCorrection {
                                        skin: skin_index,
                                    },
                                );
                            }
                            Ok::<(), DeformationAuthoringSemanticError>(())
                        })??;
                    }
                    let offset = advance_cursor(
                        &mut cursors.correction_scalars,
                        INVERSE_BIND_SCALARS,
                        "skin correction scalars",
                    )?;
                    (offset, INVERSE_BIND_SCALARS)
                }
            };
        context.add_payload(u64::from(inverse_scalar_count), u64::from(joint_count))?;
        context.add_payload(u64::from(correction_matrix_scalar_count), 0)?;
        storage.skin_plans.push(ValidatedSkinPlan {
            name_byte_offset,
            name_byte_count: skin_name_byte_count,
            joint_offset,
            joint_count,
            inverse_bind_scalar_offset,
            correction_matrix_scalar_offset,
            correction_matrix_scalar_count,
        });
    }
    Ok(())
}

fn validate_logical_morphs<C, R>(
    input: DeformationAuthoringBatch<'_>,
    default_weight_count: &mut u32,
    name_byte_count: &mut u32,
    storage: &mut DeformationValidationStorage<R>,
    context: &mut DeformationValidationContext<'_, C>,
) -> DeformationValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    let definition_count = collection_count(
        input.morph_target_definitions().len(),
        "morph target definitions",
    )?;
    let mut expected_definition_offset = 0_u32;
    for (logical_mesh, morph) in input.logical_meshes().iter().copied().enumerate() {
        let definitions = morph.definitions();
        let definition_end = u64::from(definitions.offset()) + u64::from(definitions.count());
        context.run_unit(SceneValidationWork::Entity, || {
            if definitions.offset() != expected_definition_offset {
                return Err(
                    DeformationAuthoringSemanticError::MorphDefinitionRangeOrder {
                        logical_mesh,
                        expected: expected_definition_offset,
                        actual: definitions.offset(),
                    },
                );
            }
            if definition_end > u64::from(definition_count) {
                return Err(
                    DeformationAuthoringSemanticError::MorphDefinitionRangeOutOfRange {
                        logical_mesh,
                        offset: definitions.offset(),
                        end: definition_end,
                        definition_count,
                    },
                );
            }
            let target_count = definitions.count();
            if usize::try_from(target_count).map_or(true, |target_count| {
                target_count != morph.default_weights().len()
            }) {
                return Err(
                    DeformationAuthoringSemanticError::LogicalMorphWidthMismatch {
                        logical_mesh,
                        definition_count: target_count,
                        default_count: morph.default_weights().len(),
                    },
                );
            }
            Ok::<(), DeformationAuthoringSemanticError>(())
        })??;
        let target_count = definitions.count();
        let definition_range = checked_range(definitions, input.morph_target_definitions().len())?;
        let authored_definitions = input
            .morph_target_definitions()
            .get(definition_range)
            .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "validated morph definition range",
            })?;
        let target_width = usize::try_from(target_count).map_err(|_| {
            DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "logical morph target width",
            }
        })?;
        if storage.uniqueness.capacity() < target_width {
            return Err(DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "morph identity uniqueness capacity",
            }
            .into());
        }
        storage.uniqueness.clear();
        for _ in 0..target_width {
            context.run_unit(SceneValidationWork::Traversal, || {
                storage.uniqueness.push(0);
            })?;
        }
        for (target, definition) in authored_definitions.iter().copied().enumerate() {
            let identity = definition.identity().value();
            context.run_unit(SceneValidationWork::Entity, || {
                if identity >= target_count {
                    return Err(
                        DeformationAuthoringSemanticError::MorphTargetIdentityOutOfRange {
                            logical_mesh,
                            target,
                            identity,
                            target_count,
                        },
                    );
                }
                let identity_index = usize::try_from(identity).map_err(|_| {
                    DeformationAuthoringSemanticError::AccountingOverflow {
                        dimension: "morph target identity",
                    }
                })?;
                let seen = storage.uniqueness.get_mut(identity_index).ok_or(
                    DeformationAuthoringSemanticError::AccountingOverflow {
                        dimension: "morph identity uniqueness slot",
                    },
                )?;
                if *seen != 0 {
                    return Err(
                        DeformationAuthoringSemanticError::DuplicateMorphTargetIdentity {
                            logical_mesh,
                            target,
                            identity,
                        },
                    );
                }
                *seen = 1;
                Ok::<(), DeformationAuthoringSemanticError>(())
            })??;

            let (name_byte_offset, definition_name_byte_count) = match definition.name() {
                Presence::Absent => (0, 0),
                Presence::Present(name) => {
                    let count = collection_count(name.as_str().len(), "morph target name bytes")?;
                    let offset = advance_cursor(name_byte_count, count, "morph target name bytes")?;
                    for byte in name.as_str().as_bytes() {
                        context.run_unit(SceneValidationWork::PayloadCell, || *byte)?;
                    }
                    context.add_payload_bytes(u64::from(count))?;
                    (offset, count)
                }
            };
            storage
                .morph_definition_plans
                .push(ValidatedMorphTargetDefinitionPlan {
                    logical_mesh: usize_to_u32(logical_mesh, "morph definition logical mesh")?,
                    target_ordinal: usize_to_u32(target, "morph definition target ordinal")?,
                    name_byte_offset,
                    name_byte_count: definition_name_byte_count,
                });
        }
        context.add_relations(u64::from(target_count))?;
        let default_weight_offset = advance_cursor(
            default_weight_count,
            target_count,
            "logical morph default weights",
        )?;
        for (target, weight) in morph.default_weights().iter().copied().enumerate() {
            context.run_unit(SceneValidationWork::PayloadCell, || {
                if !weight.is_finite() {
                    return Err(
                        DeformationAuthoringSemanticError::NonFiniteDefaultMorphWeight {
                            logical_mesh,
                            target,
                        },
                    );
                }
                Ok::<(), DeformationAuthoringSemanticError>(())
            })??;
        }
        context.add_payload(u64::from(target_count), 0)?;
        storage.logical_morph_plans.push(ValidatedLogicalMorphPlan {
            definitions,
            default_weight_offset,
            target_count,
            maximum_palette_joint: None,
        });
        expected_definition_offset = u32::try_from(definition_end).map_err(|_| {
            DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "morph definition range end",
            }
        })?;
    }
    context.run_unit(SceneValidationWork::Traversal, || {
        if expected_definition_offset != definition_count {
            return Err(DeformationAuthoringSemanticError::UnownedMorphDefinitions {
                owned: expected_definition_offset,
                definition_count,
            });
        }
        Ok::<(), DeformationAuthoringSemanticError>(())
    })??;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_primitives<C, R, GR>(
    input: DeformationAuthoringBatch<'_>,
    geometry: &ValidatedGeometryBatch<'_, GR>,
    influence_row_offset_count: &mut u32,
    influence_count: &mut u32,
    morph_value_count: &mut u32,
    morph_sparse_index_count: &mut u32,
    storage: &mut DeformationValidationStorage<R>,
    context: &mut DeformationValidationContext<'_, C>,
) -> DeformationValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    for (logical_mesh_index, geometry_mesh) in geometry
        .input()
        .logical_meshes()
        .iter()
        .copied()
        .enumerate()
    {
        let logical_plan = storage
            .logical_morph_plans
            .get(logical_mesh_index)
            .copied()
            .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "logical morph plan",
            })?;
        let primitive_range = checked_range(
            geometry_mesh.primitives(),
            usize::try_from(geometry.primitive_count()).map_err(|_| {
                DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "geometry primitive count",
                }
            })?,
        )?;
        let mut maximum = None;
        for primitive_index in primitive_range {
            let authored = *input.primitives().get(primitive_index).ok_or(
                DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "primitive deformation input",
                },
            )?;
            let geometry_primitive = geometry.primitive(primitive_index).ok_or(
                DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "validated geometry primitive",
                },
            )?;
            let candidate = validate_primitive(
                primitive_index,
                authored,
                geometry_primitive.vertex_count(),
                logical_plan.target_count,
                influence_row_offset_count,
                influence_count,
                morph_value_count,
                morph_sparse_index_count,
                storage,
                context,
            )?;
            if candidate.is_some_and(|candidate| {
                maximum.is_none_or(|current: PaletteJointLocation| {
                    candidate.palette_joint > current.palette_joint
                })
            }) {
                maximum = candidate;
            }
        }
        storage
            .logical_morph_plans
            .get_mut(logical_mesh_index)
            .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "logical morph palette summary",
            })?
            .maximum_palette_joint = maximum;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_primitive<C, R>(
    primitive_index: usize,
    authored: PrimitiveDeformationAuthoring<'_>,
    vertex_count: u32,
    target_count: u32,
    influence_row_offset_count: &mut u32,
    influence_count: &mut u32,
    morph_value_count: &mut u32,
    morph_sparse_index_count: &mut u32,
    storage: &mut DeformationValidationStorage<R>,
    context: &mut DeformationValidationContext<'_, C>,
) -> DeformationValidationResult<Option<PaletteJointLocation>, C>
where
    C: CanonicalAuthoringControl,
{
    let influences = context.run_unit(SceneValidationWork::Entity, || {
        let influences = authored.influences();
        if let Presence::Present(rows) = influences {
            let expected_offsets = u64::from(vertex_count) + 1;
            if u64::try_from(rows.vertex_offsets().len()) != Ok(expected_offsets) {
                return Err(
                    DeformationAuthoringSemanticError::InfluenceOffsetCountMismatch {
                        primitive: primitive_index,
                        expected: expected_offsets,
                        actual: rows.vertex_offsets().len(),
                    },
                );
            }
        }
        if authored.morph_targets().len()
            != usize::try_from(target_count).map_err(|_| {
                DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "logical morph target count",
                }
            })?
        {
            return Err(
                DeformationAuthoringSemanticError::PrimitiveMorphTargetCountMismatch {
                    primitive: primitive_index,
                    expected: target_count,
                    actual: authored.morph_targets().len(),
                },
            );
        }
        Ok::<_, DeformationAuthoringSemanticError>(influences)
    })??;
    let (row_offset_offset, row_offset_count, influence_offset, primitive_influence_count, maximum) =
        match influences {
            Presence::Absent => (0, 0, 0, 0, None),
            Presence::Present(rows) => validate_influence_rows(
                primitive_index,
                rows,
                influence_row_offset_count,
                influence_count,
                &mut storage.uniqueness,
                context,
            )?,
        };

    let target_offset = collection_count(storage.morph_target_plans.len(), "morph target offset")?;
    for (target_index, target) in authored.morph_targets().iter().copied().enumerate() {
        let deltas = context.run_unit(SceneValidationWork::Entity, || target.deltas())?;
        let delta_offset = collection_count(storage.morph_delta_plans.len(), "morph delta offset")?;
        let mut previous_semantic = None;
        for (delta_index, delta) in deltas.iter().copied().enumerate() {
            let ordinal = context.run_unit(SceneValidationWork::Entity, || {
                let ordinal = morph_semantic_ordinal(delta.semantic());
                if let Some(previous) = previous_semantic {
                    if ordinal == previous {
                        return Err(DeformationAuthoringSemanticError::DuplicateMorphSemantic {
                            primitive: primitive_index,
                            target: target_index,
                            delta: delta_index,
                            semantic: delta.semantic(),
                        });
                    }
                    if ordinal < previous {
                        return Err(DeformationAuthoringSemanticError::MorphSemanticOrder {
                            primitive: primitive_index,
                            target: target_index,
                            delta: delta_index,
                        });
                    }
                }
                validate_morph_delta_shape(primitive_index, target_index, delta, vertex_count)?;
                Ok::<u8, DeformationAuthoringSemanticError>(ordinal)
            })??;
            previous_semantic = Some(ordinal);
            let plan = validate_morph_delta(
                primitive_index,
                target_index,
                delta,
                vertex_count,
                morph_value_count,
                morph_sparse_index_count,
                context,
            )?;
            storage.morph_delta_plans.push(plan);
        }
        let delta_count = collection_count(deltas.len(), "morph target deltas")?;
        context.add_relations(u64::from(delta_count))?;
        storage.morph_target_plans.push(ValidatedMorphTargetPlan {
            primitive_index: usize_to_u32(primitive_index, "primitive index")?,
            target_ordinal: usize_to_u32(target_index, "morph target ordinal")?,
            deltas: AuthoringRange::new(delta_offset, delta_count),
        });
    }
    context.add_relations(u64::from(target_count))?;
    storage
        .primitive_plans
        .push(ValidatedPrimitiveDeformationPlan {
            influence_row_offset_offset: row_offset_offset,
            influence_row_offset_count: row_offset_count,
            influence_offset,
            influence_count: primitive_influence_count,
            morph_targets: AuthoringRange::new(target_offset, target_count),
        });
    Ok(maximum)
}

fn validate_influence_rows<C>(
    primitive: usize,
    rows: InfluenceRowsAuthoring<'_>,
    row_offset_cursor: &mut u32,
    influence_cursor: &mut u32,
    uniqueness: &mut Vec<u8>,
    context: &mut DeformationValidationContext<'_, C>,
) -> DeformationValidationResult<(u32, u32, u32, u32, Option<PaletteJointLocation>), C>
where
    C: CanonicalAuthoringControl,
{
    let row_offset_count = collection_count(rows.vertex_offsets().len(), "influence row offsets")?;
    let influence_count = collection_count(rows.influences().len(), "influences")?;
    let row_offset_offset =
        advance_cursor(row_offset_cursor, row_offset_count, "influence row offsets")?;
    let influence_offset = advance_cursor(influence_cursor, influence_count, "influence cells")?;
    let offset_len = rows.vertex_offsets().len();
    for (position, offset) in rows.vertex_offsets().iter().copied().enumerate() {
        context.run_unit(SceneValidationWork::PayloadCell, || {
            if position == 0 && offset != 0 {
                return Err(DeformationAuthoringSemanticError::InfluenceFirstOffset {
                    primitive,
                    actual: offset,
                });
            }
            if offset > influence_count {
                return Err(
                    DeformationAuthoringSemanticError::InfluenceOffsetOutOfRange {
                        primitive,
                        position,
                        offset,
                        influence_count,
                    },
                );
            }
            if position + 1 == offset_len && offset != influence_count {
                return Err(DeformationAuthoringSemanticError::InfluenceFinalOffset {
                    primitive,
                    expected: influence_count,
                    actual: offset,
                });
            }
            Ok::<(), DeformationAuthoringSemanticError>(())
        })??;
    }

    let mut maximum = None;
    for (vertex, offsets) in rows.vertex_offsets().windows(2).enumerate() {
        let (start, row, row_width) =
            context.run_unit(SceneValidationWork::Traversal, || {
                if offsets[0] > offsets[1] {
                    return Err(DeformationAuthoringSemanticError::InfluenceOffsetOrder {
                        primitive,
                        position: vertex + 1,
                        previous: offsets[0],
                        current: offsets[1],
                    });
                }
                let start = usize::try_from(offsets[0]).map_err(|_| {
                    DeformationAuthoringSemanticError::AccountingOverflow {
                        dimension: "influence row start",
                    }
                })?;
                let end = usize::try_from(offsets[1]).map_err(|_| {
                    DeformationAuthoringSemanticError::AccountingOverflow {
                        dimension: "influence row end",
                    }
                })?;
                let row = rows.influences().get(start..end).ok_or(
                    DeformationAuthoringSemanticError::AccountingOverflow {
                        dimension: "validated influence row",
                    },
                )?;
                let row_width = collection_count(row.len(), "influence row width")?;
                if uniqueness.capacity() < row.len() {
                    return Err(DeformationAuthoringSemanticError::AccountingOverflow {
                        dimension: "influence source-order uniqueness capacity",
                    });
                }
                Ok::<_, DeformationAuthoringSemanticError>((start, row, row_width))
            })??;
        uniqueness.clear();
        for _ in 0..row.len() {
            context.run_unit(SceneValidationWork::Traversal, || uniqueness.push(0))?;
        }
        for (relative, influence) in row.iter().copied().enumerate() {
            let influence_index = start.checked_add(relative).ok_or(
                DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "influence index",
                },
            )?;
            context.run_unit(SceneValidationWork::PayloadCell, || {
                if !influence.weight().is_finite() {
                    return Err(
                        DeformationAuthoringSemanticError::NonFiniteInfluenceWeight {
                            primitive,
                            influence: influence_index,
                        },
                    );
                }
                if influence.weight() < 0.0 {
                    return Err(DeformationAuthoringSemanticError::NegativeInfluenceWeight {
                        primitive,
                        influence: influence_index,
                    });
                }
                Ok::<(), DeformationAuthoringSemanticError>(())
            })??;
            let palette_joint =
                context.run_unit(SceneValidationWork::PayloadCell, || influence.joint())?;
            let candidate = context.run_unit(SceneValidationWork::PayloadCell, || {
                if influence.source_order() >= row_width {
                    return Err(
                        DeformationAuthoringSemanticError::InfluenceSourceOrderOutOfRange {
                            primitive,
                            vertex: usize_to_u32(vertex, "influence vertex")?,
                            influence: influence_index,
                            source_order: influence.source_order(),
                            row_width,
                        },
                    );
                }
                let source_order = usize::try_from(influence.source_order()).map_err(|_| {
                    DeformationAuthoringSemanticError::AccountingOverflow {
                        dimension: "influence source order",
                    }
                })?;
                let seen = uniqueness.get_mut(source_order).ok_or(
                    DeformationAuthoringSemanticError::AccountingOverflow {
                        dimension: "influence source-order uniqueness slot",
                    },
                )?;
                if *seen != 0 {
                    return Err(
                        DeformationAuthoringSemanticError::DuplicateInfluenceSourceOrder {
                            primitive,
                            vertex: usize_to_u32(vertex, "influence vertex")?,
                            source_order: influence.source_order(),
                        },
                    );
                }
                *seen = 1;
                Ok::<PaletteJointLocation, DeformationAuthoringSemanticError>(
                    PaletteJointLocation {
                        primitive: usize_to_u32(primitive, "palette primitive")?,
                        influence: usize_to_u32(influence_index, "palette influence")?,
                        palette_joint,
                    },
                )
            })??;
            if maximum.is_none_or(|current: PaletteJointLocation| {
                candidate.palette_joint > current.palette_joint
            }) {
                maximum = Some(candidate);
            }
        }
    }
    context.add_payload(
        u64::from(influence_count),
        u64::from(row_offset_count)
            .checked_add(u64::from(influence_count).checked_mul(2).ok_or(
                DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "influence u32 payload",
                },
            )?)
            .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "influence u32 payload",
            })?,
    )?;
    Ok((
        row_offset_offset,
        row_offset_count,
        influence_offset,
        influence_count,
        maximum,
    ))
}

fn validate_morph_delta_shape(
    primitive: usize,
    target: usize,
    delta: MorphDeltaAuthoring<'_>,
    vertex_count: u32,
) -> Result<(), DeformationAuthoringSemanticError> {
    let semantic = delta.semantic();
    match delta.payload() {
        MorphDeltaPayloadAuthoring::Dense { values } => {
            let expected = u64::from(vertex_count)
                .checked_mul(u64::from(MORPH_DELTA_LANES))
                .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "dense morph scalar count",
                })?;
            if u64::try_from(values.len()) != Ok(expected) {
                return Err(DeformationAuthoringSemanticError::DenseMorphScalarCount {
                    primitive,
                    target,
                    semantic,
                    expected,
                    actual: values.len(),
                });
            }
        }
        MorphDeltaPayloadAuthoring::Sparse {
            logical_count,
            indices,
            values,
        } => {
            if logical_count != vertex_count {
                return Err(DeformationAuthoringSemanticError::SparseMorphLogicalCount {
                    primitive,
                    target,
                    semantic,
                    expected: vertex_count,
                    actual: logical_count,
                });
            }
            let expected = u64::try_from(indices.len())
                .ok()
                .and_then(|count| count.checked_mul(u64::from(MORPH_DELTA_LANES)))
                .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "sparse morph scalar count",
                })?;
            if u64::try_from(values.len()) != Ok(expected) {
                return Err(DeformationAuthoringSemanticError::SparseMorphScalarCount {
                    primitive,
                    target,
                    semantic,
                    index_count: indices.len(),
                    actual: values.len(),
                });
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_morph_delta<C>(
    primitive: usize,
    target: usize,
    delta: MorphDeltaAuthoring<'_>,
    vertex_count: u32,
    value_cursor: &mut u32,
    sparse_index_cursor: &mut u32,
    context: &mut DeformationValidationContext<'_, C>,
) -> DeformationValidationResult<ValidatedMorphDeltaPlan, C>
where
    C: CanonicalAuthoringControl,
{
    let semantic = delta.semantic();
    let (payload_kind, logical_count, explicit_count, values, sparse_indices) =
        match delta.payload() {
            MorphDeltaPayloadAuthoring::Dense { values } => (
                DeformationMorphPayloadKind::Dense,
                vertex_count,
                vertex_count,
                values,
                &[][..],
            ),
            MorphDeltaPayloadAuthoring::Sparse {
                logical_count,
                indices,
                values,
            } => {
                for (element, index) in indices.iter().copied().enumerate() {
                    context.run_unit(SceneValidationWork::PayloadCell, || {
                        if index >= logical_count {
                            return Err(
                                DeformationAuthoringSemanticError::SparseMorphIndexOutOfRange {
                                    primitive,
                                    target,
                                    semantic,
                                    element,
                                    index,
                                    logical_count,
                                },
                            );
                        }
                        Ok::<(), DeformationAuthoringSemanticError>(())
                    })??;
                }
                for (element, pair) in indices.windows(2).enumerate() {
                    context.run_unit(SceneValidationWork::Traversal, || {
                        if pair[0] >= pair[1] {
                            return Err(DeformationAuthoringSemanticError::SparseMorphIndexOrder {
                                primitive,
                                target,
                                semantic,
                                element: element + 1,
                            });
                        }
                        Ok::<(), DeformationAuthoringSemanticError>(())
                    })??;
                }
                (
                    DeformationMorphPayloadKind::Sparse,
                    logical_count,
                    collection_count(indices.len(), "sparse morph indices")?,
                    values,
                    indices,
                )
            }
        };

    for (scalar, value) in values.iter().copied().enumerate() {
        context.run_unit(SceneValidationWork::PayloadCell, || {
            if !value.is_finite() {
                return Err(DeformationAuthoringSemanticError::NonFiniteMorphDelta {
                    primitive,
                    target,
                    semantic,
                    scalar,
                });
            }
            Ok::<(), DeformationAuthoringSemanticError>(())
        })??;
    }
    let value_count = collection_count(values.len(), "morph delta values")?;
    let sparse_index_count = collection_count(sparse_indices.len(), "morph sparse indices")?;
    let value_offset = advance_cursor(value_cursor, value_count, "morph delta values")?;
    let sparse_index_offset = advance_cursor(
        sparse_index_cursor,
        sparse_index_count,
        "morph sparse indices",
    )?;
    context.add_payload(u64::from(value_count), u64::from(sparse_index_count))?;
    Ok(ValidatedMorphDeltaPlan {
        semantic,
        payload_kind,
        logical_count,
        explicit_count,
        value_offset,
        value_count,
        sparse_index_offset,
        sparse_index_count,
    })
}

fn validate_mesh_instances<C, R, GR>(
    input: DeformationAuthoringBatch<'_>,
    geometry: &ValidatedGeometryBatch<'_, GR>,
    morph_weight_cursor: &mut u32,
    storage: &mut DeformationValidationStorage<R>,
    context: &mut DeformationValidationContext<'_, C>,
) -> DeformationValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    for (instance_index, authored) in input.mesh_instances().iter().copied().enumerate() {
        let (logical_mesh, skin) = context.run_unit(SceneValidationWork::Entity, || {
            let geometry_instance = *geometry
                .input()
                .mesh_instances()
                .get(instance_index)
                .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "geometry mesh instance",
                })?;
            let logical_mesh = geometry_instance.logical_mesh().index();
            let logical_plan = storage
                .logical_morph_plans
                .get(usize::try_from(logical_mesh).map_err(|_| {
                    DeformationAuthoringSemanticError::AccountingOverflow {
                        dimension: "instance logical mesh index",
                    }
                })?)
                .copied()
                .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "instance logical morph plan",
                })?;
            let skin = match authored.skin() {
                Presence::Absent => None,
                Presence::Present(skin) => {
                    let skin_count = collection_count(input.skins().len(), "skins")?;
                    if skin.index() >= skin_count {
                        return Err(DeformationAuthoringSemanticError::InstanceSkinOutOfRange {
                            instance: instance_index,
                            skin: skin.index(),
                            skin_count,
                        });
                    }
                    let skin_plan = storage
                        .skin_plans
                        .get(usize::try_from(skin.index()).map_err(|_| {
                            DeformationAuthoringSemanticError::AccountingOverflow {
                                dimension: "instance skin index",
                            }
                        })?)
                        .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                            dimension: "instance skin plan",
                        })?;
                    if let Some(maximum) = logical_plan.maximum_palette_joint
                        && maximum.palette_joint >= skin_plan.joint_count
                    {
                        return Err(DeformationAuthoringSemanticError::PaletteJointOutOfRange {
                            instance: instance_index,
                            logical_mesh,
                            skin: skin.index(),
                            joint_count: skin_plan.joint_count,
                            primitive: maximum.primitive,
                            influence: maximum.influence,
                            palette_joint: maximum.palette_joint,
                        });
                    }
                    Some(skin)
                }
            };
            if let Presence::Present(weights) = authored.morph_weights()
                && weights.len()
                    != usize::try_from(logical_plan.target_count).map_err(|_| {
                        DeformationAuthoringSemanticError::AccountingOverflow {
                            dimension: "instance morph target count",
                        }
                    })?
            {
                return Err(DeformationAuthoringSemanticError::InstanceMorphWidth {
                    instance: instance_index,
                    expected: logical_plan.target_count,
                    actual: weights.len(),
                });
            }
            Ok::<_, DeformationAuthoringSemanticError>((logical_mesh, skin))
        })??;

        if skin.is_some() {
            context.add_relations(1)?;
        }

        let (morph_weight_offset, morph_weight_count) = match authored.morph_weights() {
            Presence::Absent => (0, 0),
            Presence::Present(weights) => {
                for (target, weight) in weights.iter().copied().enumerate() {
                    context.run_unit(SceneValidationWork::PayloadCell, || {
                        if !weight.is_finite() {
                            return Err(
                                DeformationAuthoringSemanticError::NonFiniteInstanceMorphWeight {
                                    instance: instance_index,
                                    target,
                                },
                            );
                        }
                        Ok::<(), DeformationAuthoringSemanticError>(())
                    })??;
                }
                let count = collection_count(weights.len(), "instance morph weights")?;
                let offset = advance_cursor(morph_weight_cursor, count, "instance morph weights")?;
                context.add_payload(u64::from(count), 0)?;
                (offset, count)
            }
        };
        storage
            .mesh_instance_plans
            .push(ValidatedMeshInstanceDeformationPlan {
                logical_mesh,
                skin,
                morph_weight_offset,
                morph_weight_count,
            });
    }
    Ok(())
}

fn deformation_scratch_shape(
    input: DeformationAuthoringBatch<'_>,
) -> Result<DeformationScratchShape, DeformationAuthoringSemanticError> {
    let morph_definition_count = input.morph_target_definitions().len();
    let mut morph_target_count = 0_usize;
    let mut morph_delta_count = 0_usize;
    let mut joint_node_count = 0_u64;
    let mut inverse_bind_scalar_count = 0_u64;
    let mut skin_correction_scalar_count = 0_u64;
    let mut skin_name_byte_count = 0_u64;
    let mut influence_row_offset_count = 0_u64;
    let mut influence_count = 0_u64;
    let mut morph_default_weight_count = 0_u64;
    let mut morph_name_byte_count = 0_u64;
    let mut morph_value_count = 0_u64;
    let mut morph_sparse_index_count = 0_u64;
    let mut instance_morph_weight_count = 0_u64;
    let mut maximum_uniqueness_count = 0_usize;

    collection_count(input.skins().len(), "skins")?;
    collection_count(morph_definition_count, "morph target definitions")?;
    collection_count(input.logical_meshes().len(), "logical morphs")?;
    collection_count(input.primitives().len(), "primitive deformation")?;
    collection_count(input.mesh_instances().len(), "mesh instance deformation")?;
    for skin in input.skins() {
        if let Presence::Present(name) = skin.name() {
            skin_name_byte_count =
                checked_add_len(skin_name_byte_count, name.as_str().len(), "skin name bytes")?;
        }
        joint_node_count = checked_add_len(joint_node_count, skin.joints().len(), "skin joints")?;
        inverse_bind_scalar_count = inverse_bind_scalar_count
            .checked_add(
                usize_to_u64(skin.inverse_bind_matrices().len(), "inverse-bind matrices")?
                    .checked_mul(u64::from(INVERSE_BIND_SCALARS))
                    .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                        dimension: "inverse-bind scalars",
                    })?,
            )
            .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "inverse-bind scalars",
            })?;
        if skin.matrix_spaces().correction().is_some() {
            skin_correction_scalar_count = skin_correction_scalar_count
                .checked_add(u64::from(INVERSE_BIND_SCALARS))
                .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "skin correction scalars",
                })?;
        }
    }
    for definition in input.morph_target_definitions() {
        if let Presence::Present(name) = definition.name() {
            morph_name_byte_count = checked_add_len(
                morph_name_byte_count,
                name.as_str().len(),
                "morph target name bytes",
            )?;
        }
    }
    for logical in input.logical_meshes() {
        maximum_uniqueness_count = maximum_uniqueness_count.max(valid_morph_definition_width(
            logical.definitions(),
            morph_definition_count,
        ));
        morph_default_weight_count = checked_add_len(
            morph_default_weight_count,
            logical.default_weights().len(),
            "logical morph default weights",
        )?;
    }
    for primitive in input.primitives() {
        if let Presence::Present(rows) = primitive.influences() {
            maximum_uniqueness_count =
                maximum_uniqueness_count.max(maximum_valid_influence_row_width(rows));
            influence_row_offset_count = checked_add_len(
                influence_row_offset_count,
                rows.vertex_offsets().len(),
                "influence row offsets",
            )?;
            influence_count =
                checked_add_len(influence_count, rows.influences().len(), "influences")?;
        }
        morph_target_count = morph_target_count
            .checked_add(primitive.morph_targets().len())
            .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "morph target count",
            })?;
        for target in primitive.morph_targets() {
            morph_delta_count = morph_delta_count.checked_add(target.deltas().len()).ok_or(
                DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "morph delta count",
                },
            )?;
            for delta in target.deltas() {
                match delta.payload() {
                    MorphDeltaPayloadAuthoring::Dense { values } => {
                        morph_value_count =
                            checked_add_len(morph_value_count, values.len(), "morph delta values")?;
                    }
                    MorphDeltaPayloadAuthoring::Sparse {
                        indices, values, ..
                    } => {
                        morph_value_count =
                            checked_add_len(morph_value_count, values.len(), "morph delta values")?;
                        morph_sparse_index_count = checked_add_len(
                            morph_sparse_index_count,
                            indices.len(),
                            "morph sparse indices",
                        )?;
                    }
                }
            }
        }
    }
    for instance in input.mesh_instances() {
        if let Presence::Present(weights) = instance.morph_weights() {
            instance_morph_weight_count = checked_add_len(
                instance_morph_weight_count,
                weights.len(),
                "instance morph weights",
            )?;
        }
    }
    let shape = DeformationScratchShape {
        morph_definition_count,
        morph_target_count,
        morph_delta_count,
        maximum_uniqueness_count,
        joint_node_count,
        inverse_bind_scalar_count,
        skin_correction_scalar_count,
        skin_name_byte_count,
        influence_row_offset_count,
        influence_count,
        morph_default_weight_count,
        morph_name_byte_count,
        morph_value_count,
        morph_sparse_index_count,
        instance_morph_weight_count,
    };
    validate_deformation_shape_counts(shape)?;
    Ok(shape)
}

fn deformation_capacity_plans(
    input: DeformationAuthoringBatch<'_>,
    shape: DeformationScratchShape,
) -> Result<DeformationCapacityPlans, DeformationAuthoringSemanticError> {
    Ok(DeformationCapacityPlans {
        skins: scratch_plan::<ValidatedSkinPlan>(input.skins().len(), "validated skin plans")?,
        logical_morphs: scratch_plan::<ValidatedLogicalMorphPlan>(
            input.logical_meshes().len(),
            "validated logical morph plans",
        )?,
        morph_definitions: scratch_plan::<ValidatedMorphTargetDefinitionPlan>(
            shape.morph_definition_count,
            "validated morph definition plans",
        )?,
        primitives: scratch_plan::<ValidatedPrimitiveDeformationPlan>(
            input.primitives().len(),
            "validated primitive deformation plans",
        )?,
        morph_targets: scratch_plan::<ValidatedMorphTargetPlan>(
            shape.morph_target_count,
            "validated morph target plans",
        )?,
        morph_deltas: scratch_plan::<ValidatedMorphDeltaPlan>(
            shape.morph_delta_count,
            "validated morph delta plans",
        )?,
        mesh_instances: scratch_plan::<ValidatedMeshInstanceDeformationPlan>(
            input.mesh_instances().len(),
            "validated mesh instance plans",
        )?,
        uniqueness: scratch_plan::<u8>(
            shape.maximum_uniqueness_count,
            "deformation uniqueness scratch",
        )?,
    })
}

fn deformation_structural_bytes<C>(
    input: DeformationAuthoringBatch<'_>,
    context: &mut DeformationValidationContext<'_, C>,
) -> DeformationValidationResult<(u64, DeformationScratchShape), C>
where
    C: CanonicalAuthoringControl,
{
    let skin_count = collection_count(input.skins().len(), "skins")?;
    let morph_definition_count = collection_count(
        input.morph_target_definitions().len(),
        "morph target definitions",
    )?;
    let logical_count = collection_count(input.logical_meshes().len(), "logical morphs")?;
    let primitive_count = collection_count(input.primitives().len(), "primitive deformation")?;
    let instance_count = collection_count(input.mesh_instances().len(), "mesh instances")?;

    let mut maximum_uniqueness_count = 0;
    let mut joint_nodes = 0_u64;
    let mut inverse_bind_scalars = 0_u64;
    let mut skin_correction_scalars = 0_u64;
    let mut skin_name_bytes = 0_u64;
    let mut influence_row_offsets = 0_u64;
    let mut influences = 0_u64;
    let mut morph_defaults = 0_u64;
    let mut morph_name_bytes = 0_u64;
    let mut morph_values = 0_u64;
    let mut morph_sparse_indices = 0_u64;
    let mut instance_weights = 0_u64;
    let mut skin_bytes = 0_u64;
    for skin in input.skins() {
        context.run_unit(SceneValidationWork::Traversal, || {
            let name_bytes = match skin.name() {
                Presence::Absent => 0,
                Presence::Present(name) => name.as_str().len(),
            };
            skin_name_bytes = checked_add_len(skin_name_bytes, name_bytes, "skin name bytes")?;
            joint_nodes = checked_add_len(joint_nodes, skin.joints().len(), "skin joint nodes")?;
            inverse_bind_scalars = inverse_bind_scalars
                .checked_add(
                    usize_to_u64(skin.inverse_bind_matrices().len(), "inverse-bind matrices")?
                        .checked_mul(u64::from(INVERSE_BIND_SCALARS))
                        .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                            dimension: "inverse-bind scalars",
                        })?,
                )
                .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "inverse-bind scalars",
                })?;
            let has_correction = skin.matrix_spaces().correction().is_some();
            if has_correction {
                skin_correction_scalars = skin_correction_scalars
                    .checked_add(u64::from(INVERSE_BIND_SCALARS))
                    .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                        dimension: "skin correction scalars",
                    })?;
            }
            skin_bytes = skin_bytes
                .checked_add(DEFORMATION_SKIN_FIXED_BYTES)
                .and_then(|bytes| {
                    bytes.checked_add(checked_reference_slots([
                        !skin.joints().is_empty(),
                        !skin.inverse_bind_matrices().is_empty(),
                        !skin.joints().is_empty()
                            || matches!(skin.skeleton_root(), Presence::Present(_)),
                        has_correction,
                        name_bytes != 0,
                    ]))
                })
                .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "deformation skin records",
                })?;
            Ok::<(), DeformationAuthoringSemanticError>(())
        })??;
    }

    let mut morph_definition_bytes = 0_u64;
    for definition in input.morph_target_definitions() {
        context.run_unit(SceneValidationWork::Traversal, || {
            let name_bytes = match definition.name() {
                Presence::Absent => 0,
                Presence::Present(name) => name.as_str().len(),
            };
            morph_name_bytes =
                checked_add_len(morph_name_bytes, name_bytes, "morph target name bytes")?;
            morph_definition_bytes = morph_definition_bytes
                .checked_add(DEFORMATION_MORPH_DEFINITION_FIXED_BYTES)
                .and_then(|bytes| bytes.checked_add(reference_slot_bytes(name_bytes != 0)))
                .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "morph definition records",
                })?;
            Ok::<(), DeformationAuthoringSemanticError>(())
        })??;
    }

    let mut logical_bytes = 0_u64;
    for logical in input.logical_meshes() {
        context.run_unit(SceneValidationWork::Traversal, || {
            maximum_uniqueness_count = maximum_uniqueness_count.max(valid_morph_definition_width(
                logical.definitions(),
                input.morph_target_definitions().len(),
            ));
            morph_defaults = checked_add_len(
                morph_defaults,
                logical.default_weights().len(),
                "morph default weights",
            )?;
            logical_bytes = logical_bytes
                .checked_add(DEFORMATION_LOGICAL_MORPH_FIXED_BYTES)
                .and_then(|bytes| {
                    bytes.checked_add(checked_reference_slots([
                        logical.definitions().count() != 0,
                        !logical.default_weights().is_empty(),
                    ]))
                })
                .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "logical morph records",
                })?;
            Ok::<(), DeformationAuthoringSemanticError>(())
        })??;
    }

    let mut primitive_bytes = 0_u64;
    let mut target_bytes = 0_u64;
    let mut delta_bytes = 0_u64;
    let mut morph_target_count = 0_usize;
    let mut morph_delta_count = 0_usize;
    for primitive in input.primitives() {
        let rows = context.run_unit(SceneValidationWork::Traversal, || {
            morph_target_count = morph_target_count
                .checked_add(primitive.morph_targets().len())
                .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "morph target count",
                })?;
            let (has_rows, rows, row_count, influence_count) = match primitive.influences() {
                Presence::Absent => (false, None, 0, 0),
                Presence::Present(rows) => {
                    let influence_count = u32::try_from(rows.influences().len()).map_err(|_| {
                        DeformationAuthoringSemanticError::CountDomainTooLarge {
                            collection: "influences",
                        }
                    })?;
                    (
                        true,
                        Some((rows, influence_count)),
                        rows.vertex_offsets().len(),
                        rows.influences().len(),
                    )
                }
            };
            influence_row_offsets =
                checked_add_len(influence_row_offsets, row_count, "influence row offsets")?;
            influences = checked_add_len(influences, influence_count, "influences")?;
            primitive_bytes = primitive_bytes
                .checked_add(DEFORMATION_PRIMITIVE_FIXED_BYTES)
                .and_then(|bytes| {
                    bytes.checked_add(checked_reference_slots([
                        has_rows && row_count != 0,
                        has_rows && influence_count != 0,
                        has_rows && influence_count != 0,
                        has_rows && influence_count != 0,
                        !primitive.morph_targets().is_empty(),
                    ]))
                })
                .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "primitive deformation records",
                })?;
            Ok::<_, DeformationAuthoringSemanticError>(rows)
        })??;
        if let Some((rows, influence_count)) = rows {
            for pair in rows.vertex_offsets().windows(2) {
                context.run_unit(SceneValidationWork::Traversal, || {
                    if pair[0] <= pair[1]
                        && pair[1] <= influence_count
                        && let Ok(width) = usize::try_from(pair[1] - pair[0])
                    {
                        maximum_uniqueness_count = maximum_uniqueness_count.max(width);
                    }
                })?;
            }
        }
        for target in primitive.morph_targets() {
            let deltas = context.run_unit(SceneValidationWork::Traversal, || {
                morph_delta_count = morph_delta_count.checked_add(target.deltas().len()).ok_or(
                    DeformationAuthoringSemanticError::AccountingOverflow {
                        dimension: "morph delta count",
                    },
                )?;
                target_bytes = target_bytes
                    .checked_add(DEFORMATION_MORPH_TARGET_FIXED_BYTES)
                    .and_then(|bytes| {
                        bytes.checked_add(reference_slot_bytes(!target.deltas().is_empty()))
                    })
                    .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                        dimension: "morph target records",
                    })?;
                Ok::<_, DeformationAuthoringSemanticError>(target.deltas())
            })??;
            for delta in deltas {
                context.run_unit(SceneValidationWork::Traversal, || {
                    let (values, sparse) = match delta.payload() {
                        MorphDeltaPayloadAuthoring::Dense { values } => (values.len(), 0),
                        MorphDeltaPayloadAuthoring::Sparse {
                            indices, values, ..
                        } => (values.len(), indices.len()),
                    };
                    morph_values = checked_add_len(morph_values, values, "morph values")?;
                    morph_sparse_indices =
                        checked_add_len(morph_sparse_indices, sparse, "morph sparse indices")?;
                    delta_bytes = delta_bytes
                        .checked_add(DEFORMATION_MORPH_DELTA_FIXED_BYTES)
                        .and_then(|bytes| {
                            bytes.checked_add(checked_reference_slots([values != 0, sparse != 0]))
                        })
                        .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                            dimension: "morph delta records",
                        })?;
                    Ok::<(), DeformationAuthoringSemanticError>(())
                })??;
            }
        }
    }

    let target_count = collection_count(morph_target_count, "morph targets")?;
    let delta_count = collection_count(morph_delta_count, "morph deltas")?;

    let mut instance_bytes = 0_u64;
    for instance in input.mesh_instances() {
        context.run_unit(SceneValidationWork::Traversal, || {
            let weights = match instance.morph_weights() {
                Presence::Absent => 0,
                Presence::Present(weights) => weights.len(),
            };
            instance_weights =
                checked_add_len(instance_weights, weights, "instance morph weights")?;
            instance_bytes = instance_bytes
                .checked_add(DEFORMATION_MESH_INSTANCE_FIXED_BYTES)
                .and_then(|bytes| {
                    bytes.checked_add(checked_reference_slots([
                        matches!(instance.skin(), Presence::Present(_)),
                        weights != 0,
                    ]))
                })
                .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                    dimension: "mesh instance deformation records",
                })?;
            Ok::<(), DeformationAuthoringSemanticError>(())
        })??;
    }

    let directory_reference_bytes = checked_reference_slots([
        skin_count != 0,
        joint_nodes != 0,
        inverse_bind_scalars != 0,
        logical_count != 0,
        primitive_count != 0,
        influence_row_offsets != 0,
        influences != 0,
        influences != 0,
        influences != 0,
        target_count != 0,
        delta_count != 0,
        morph_defaults != 0,
        morph_values != 0,
        morph_sparse_indices != 0,
        instance_count != 0,
        instance_weights != 0,
        skin_correction_scalars != 0,
        morph_definition_count != 0,
        morph_name_bytes != 0,
        skin_name_bytes != 0,
    ]);
    let bytes = checked_sum_bytes(
        [
            DEFORMATION_DIRECTORY_FIXED_BYTES,
            directory_reference_bytes,
            skin_bytes,
            logical_bytes,
            morph_definition_bytes,
            primitive_bytes,
            target_bytes,
            delta_bytes,
            instance_bytes,
        ],
        "canonical deformation structural footprint",
    )?;
    let shape = DeformationScratchShape {
        morph_definition_count: input.morph_target_definitions().len(),
        morph_target_count,
        morph_delta_count,
        maximum_uniqueness_count,
        joint_node_count: joint_nodes,
        inverse_bind_scalar_count: inverse_bind_scalars,
        skin_correction_scalar_count: skin_correction_scalars,
        skin_name_byte_count: skin_name_bytes,
        influence_row_offset_count: influence_row_offsets,
        influence_count: influences,
        morph_default_weight_count: morph_defaults,
        morph_name_byte_count: morph_name_bytes,
        morph_value_count: morph_values,
        morph_sparse_index_count: morph_sparse_indices,
        instance_morph_weight_count: instance_weights,
    };
    validate_deformation_shape_counts(shape)?;
    Ok((bytes, shape))
}

fn validate_deformation_shape_counts(
    shape: DeformationScratchShape,
) -> Result<(), DeformationAuthoringSemanticError> {
    collection_count(shape.morph_definition_count, "morph target definitions")?;
    collection_count(shape.morph_target_count, "morph targets")?;
    collection_count(shape.morph_delta_count, "morph deltas")?;
    for (collection, count) in [
        ("skin joint nodes", shape.joint_node_count),
        ("inverse-bind scalars", shape.inverse_bind_scalar_count),
        (
            "skin correction scalars",
            shape.skin_correction_scalar_count,
        ),
        ("skin name bytes", shape.skin_name_byte_count),
        ("influence row offsets", shape.influence_row_offset_count),
        ("influences", shape.influence_count),
        (
            "logical morph default weights",
            shape.morph_default_weight_count,
        ),
        ("morph target name bytes", shape.morph_name_byte_count),
        ("morph delta values", shape.morph_value_count),
        ("morph sparse indices", shape.morph_sparse_index_count),
        ("instance morph weights", shape.instance_morph_weight_count),
    ] {
        u32::try_from(count)
            .map_err(|_| DeformationAuthoringSemanticError::CountDomainTooLarge { collection })?;
    }
    Ok(())
}

fn require_geometry_cardinality(
    table: &'static str,
    expected: u32,
    actual: usize,
) -> Result<(), DeformationAuthoringSemanticError> {
    if usize::try_from(expected) == Ok(actual) {
        Ok(())
    } else {
        Err(
            DeformationAuthoringSemanticError::GeometryTableCardinality {
                table,
                expected,
                actual,
            },
        )
    }
}

fn require_compiled_count(
    dimension: &'static str,
    actual: u32,
    expected: u64,
) -> Result<(), DeformationAuthoringSemanticError> {
    if u64::from(actual) == expected {
        Ok(())
    } else {
        Err(DeformationAuthoringSemanticError::AccountingOverflow { dimension })
    }
}

fn require_compiled_table_count(
    dimension: &'static str,
    actual: usize,
    expected: usize,
) -> Result<(), DeformationAuthoringSemanticError> {
    if actual == expected {
        Ok(())
    } else {
        Err(DeformationAuthoringSemanticError::AccountingOverflow { dimension })
    }
}

fn has_canonical_skin_base_spaces(spaces: SkinMatrixSpaces) -> bool {
    let canonical = SkinMatrixSpaces::canonical();
    spaces.input_position() == canonical.input_position()
        && spaces.inverse_bind_from() == canonical.inverse_bind_from()
        && spaces.inverse_bind_to() == canonical.inverse_bind_to()
        && spaces.current_joint_from() == canonical.current_joint_from()
        && spaces.current_joint_to() == canonical.current_joint_to()
        && spaces.output_position() == canonical.output_position()
}

fn maximum_valid_influence_row_width(rows: InfluenceRowsAuthoring<'_>) -> usize {
    let Ok(influence_count) = u32::try_from(rows.influences().len()) else {
        return 0;
    };
    rows.vertex_offsets()
        .windows(2)
        .filter_map(|pair| {
            (pair[0] <= pair[1] && pair[1] <= influence_count)
                .then(|| usize::try_from(pair[1] - pair[0]).ok())
                .flatten()
        })
        .max()
        .map_or(0, |width| width)
}

fn valid_morph_definition_width(range: AuthoringRange, definition_count: usize) -> usize {
    let Ok(definition_count) = u64::try_from(definition_count) else {
        return 0;
    };
    let end = u64::from(range.offset()) + u64::from(range.count());
    if end <= definition_count {
        usize::try_from(range.count()).map_or(0, |width| width)
    } else {
        0
    }
}

const fn morph_semantic_ordinal(semantic: MorphDeltaSemantic) -> u8 {
    match semantic {
        MorphDeltaSemantic::Position => 0,
        MorphDeltaSemantic::Normal => 1,
        MorphDeltaSemantic::Tangent => 2,
    }
}

fn advance_cursor(
    cursor: &mut u32,
    count: u32,
    dimension: &'static str,
) -> Result<u32, DeformationAuthoringSemanticError> {
    let start = *cursor;
    *cursor = cursor
        .checked_add(count)
        .ok_or(DeformationAuthoringSemanticError::AccountingOverflow { dimension })?;
    Ok(start)
}

fn scratch_plan<T>(
    count: usize,
    resource: &'static str,
) -> Result<CapacityPlan, DeformationAuthoringSemanticError> {
    CapacityPlan::for_type::<T>(0, 0, count)
        .map_err(|cause| DeformationAuthoringSemanticError::ScratchCapacity { resource, cause })
}

fn scratch_envelope(
    plan: CapacityPlan,
    resource: &'static str,
) -> Result<u64, DeformationAuthoringSemanticError> {
    plan.envelope_delta_bytes()
        .map_err(|cause| DeformationAuthoringSemanticError::ScratchCapacity { resource, cause })
}

fn checked_range(
    range: AuthoringRange,
    table_len: usize,
) -> Result<std::ops::Range<usize>, DeformationAuthoringSemanticError> {
    let start = usize::try_from(range.offset()).map_err(|_| {
        DeformationAuthoringSemanticError::AccountingOverflow {
            dimension: "authoring range offset",
        }
    })?;
    let count = usize::try_from(range.count()).map_err(|_| {
        DeformationAuthoringSemanticError::AccountingOverflow {
            dimension: "authoring range count",
        }
    })?;
    let end =
        start
            .checked_add(count)
            .ok_or(DeformationAuthoringSemanticError::AccountingOverflow {
                dimension: "authoring range end",
            })?;
    if end > table_len {
        return Err(DeformationAuthoringSemanticError::AccountingOverflow {
            dimension: "validated authoring range",
        });
    }
    Ok(start..end)
}

fn collection_count(
    count: usize,
    collection: &'static str,
) -> Result<u32, DeformationAuthoringSemanticError> {
    u32::try_from(count)
        .map_err(|_| DeformationAuthoringSemanticError::CountDomainTooLarge { collection })
}

fn usize_to_u32(
    value: usize,
    dimension: &'static str,
) -> Result<u32, DeformationAuthoringSemanticError> {
    u32::try_from(value)
        .map_err(|_| DeformationAuthoringSemanticError::AccountingOverflow { dimension })
}

fn usize_to_u64(
    value: usize,
    dimension: &'static str,
) -> Result<u64, DeformationAuthoringSemanticError> {
    u64::try_from(value)
        .map_err(|_| DeformationAuthoringSemanticError::AccountingOverflow { dimension })
}

fn checked_add_len(
    total: u64,
    len: usize,
    dimension: &'static str,
) -> Result<u64, DeformationAuthoringSemanticError> {
    total
        .checked_add(usize_to_u64(len, dimension)?)
        .ok_or(DeformationAuthoringSemanticError::AccountingOverflow { dimension })
}

fn checked_bytes(
    count: u64,
    width: u64,
    dimension: &'static str,
) -> Result<u64, DeformationAuthoringSemanticError> {
    count
        .checked_mul(width)
        .ok_or(DeformationAuthoringSemanticError::AccountingOverflow { dimension })
}

fn checked_sum<const N: usize>(
    values: [u64; N],
    dimension: &'static str,
) -> Result<u64, DeformationAuthoringSemanticError> {
    values.into_iter().try_fold(0_u64, |total, value| {
        total
            .checked_add(value)
            .ok_or(DeformationAuthoringSemanticError::AccountingOverflow { dimension })
    })
}

fn checked_sum_bytes<const N: usize>(
    values: [u64; N],
    dimension: &'static str,
) -> Result<u64, DeformationAuthoringSemanticError> {
    checked_sum(values, dimension)
}

const fn reference_slot_bytes(has_reference: bool) -> u64 {
    if has_reference {
        LOGICAL_REFERENCE_BYTES
    } else {
        LOGICAL_U8_BYTES
    }
}

fn checked_reference_slots<const N: usize>(references: [bool; N]) -> u64 {
    references.into_iter().map(reference_slot_bytes).sum()
}

fn enforce_limit<E>(
    limit: DeformationAuthoringLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), DeformationAuthoringFailure<E>>
where
    E: Error + Send + Sync + 'static,
{
    if actual <= maximum {
        Ok(())
    } else {
        Err(DeformationAuthoringSemanticError::LimitExceeded {
            limit,
            actual,
            maximum,
        }
        .into())
    }
}
