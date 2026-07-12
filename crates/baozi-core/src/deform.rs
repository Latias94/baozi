use crate::attribute::{AttributeBuffer, AttributePayload, AttributePayloadError};
use crate::entity::EntityName;
use crate::identity::{MorphTargetLocalIndex, NodeIndex};
use crate::math::Mat4;
use crate::provenance::Provenance;
use baozi_operation::CapacityPlan;
use std::collections::HashSet;
use thiserror::Error;

/// Maximum allocator-retained bytes in one owned deformation collection.
pub const MAX_DEFORM_COLLECTION_BYTES: usize = 64 * 1024 * 1024;

/// Maximum vertices represented by one compressed influence stream.
pub const MAX_INFLUENCE_VERTICES: usize = 4_194_303;

/// Maximum flat influence cells retained by one primitive.
pub const MAX_INFLUENCE_CELLS: usize = 4_194_304;

/// Maximum joints retained by one skin palette.
pub const MAX_SKIN_JOINTS: usize = 524_288;

/// Maximum allocator-retained bytes in one morph delta or primitive target.
pub const MAX_MORPH_DELTA_RETAINED_BYTES: usize = 64 * 1024 * 1024;

/// One owned deformation resource subject to count and retained-byte limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeformBuildResource {
    InfluenceOffsets,
    Influences,
    SkinJoints,
    SkinInverseBindMatrices,
    MorphDelta,
    PrimitiveMorphTarget,
}

/// A bounded collection dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeformLimitDimension {
    Count,
    Bytes,
}

/// A checked-size, hard-limit, or allocation failure while owning deformation data.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum DeformCollectionError {
    #[error("{resource:?} size overflows for {count} elements")]
    SizeOverflow {
        resource: DeformBuildResource,
        count: usize,
    },

    #[error("{resource:?} {dimension:?} limit exceeded: requested {requested}, maximum {maximum}")]
    LimitExceeded {
        resource: DeformBuildResource,
        dimension: DeformLimitDimension,
        requested: usize,
        maximum: usize,
    },

    #[error("failed to reserve {resource:?}")]
    Allocation { resource: DeformBuildResource },
}

/// One source-preserving skin influence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Influence {
    palette_joint: u32,
    weight: f32,
    source_order: u32,
}

impl Influence {
    /// Creates an uncommitted influence cell.
    ///
    /// Weight validity is checked when the cell is committed to an
    /// [`InfluenceRows`] value so an error can identify its exact row and cell.
    pub const fn new(palette_joint: u32, weight: f32, source_order: u32) -> Self {
        Self {
            palette_joint,
            weight,
            source_order,
        }
    }

    pub const fn palette_joint(self) -> u32 {
        self.palette_joint
    }

    pub const fn weight(self) -> f32 {
        self.weight
    }

    pub const fn source_order(self) -> u32 {
        self.source_order
    }
}

/// Variable-length skin influences stored as compressed vertex rows.
#[derive(Debug, PartialEq)]
pub struct InfluenceRows {
    vertex_count: u32,
    vertex_offsets: Vec<u32>,
    influences: Vec<Influence>,
    retained_cell_capacity: usize,
    retained_allocation_bytes: usize,
}

impl InfluenceRows {
    /// Validates and takes ownership of compressed influence rows.
    pub fn try_new<Offsets, Influences>(
        vertex_count: u32,
        vertex_offsets: Offsets,
        influences: Influences,
    ) -> Result<Self, InfluenceRowsError>
    where
        Offsets: IntoIterator<Item = u32>,
        Influences: IntoIterator<Item = Influence>,
    {
        let expected_offset_count = vertex_count
            .checked_add(1)
            .ok_or(InfluenceRowsError::VertexCountOverflow { vertex_count })?;
        let expected_offset_count = usize::try_from(expected_offset_count)
            .map_err(|_| InfluenceRowsError::VertexCountOverflow { vertex_count })?;
        preflight_deform_collection::<u32>(
            DeformBuildResource::InfluenceOffsets,
            expected_offset_count,
            MAX_INFLUENCE_VERTICES + 1,
            MAX_DEFORM_COLLECTION_BYTES,
        )?;
        let vertex_offsets = try_collect_deform(
            vertex_offsets,
            DeformBuildResource::InfluenceOffsets,
            MAX_INFLUENCE_VERTICES + 1,
            MAX_DEFORM_COLLECTION_BYTES,
        )?;
        let influences = try_collect_deform(
            influences,
            DeformBuildResource::Influences,
            MAX_INFLUENCE_CELLS,
            MAX_DEFORM_COLLECTION_BYTES,
        )?;

        if vertex_offsets.len() != expected_offset_count {
            return Err(InfluenceRowsError::OffsetCountMismatch {
                expected: expected_offset_count,
                actual: vertex_offsets.len(),
            });
        }

        let influence_count = u32::try_from(influences.len()).map_err(|_| {
            InfluenceRowsError::InfluenceCountOverflow {
                influence_count: influences.len(),
            }
        })?;

        let first_offset = vertex_offsets.first().copied().unwrap_or_default();
        if first_offset != 0 {
            return Err(InfluenceRowsError::FirstOffsetNotZero {
                actual: first_offset,
            });
        }

        for (index, &offset) in vertex_offsets.iter().enumerate() {
            if offset > influence_count {
                return Err(InfluenceRowsError::OffsetOutOfRange {
                    index,
                    offset,
                    influence_count,
                });
            }
        }

        for (previous_index, offsets) in vertex_offsets.windows(2).enumerate() {
            let previous = offsets[0];
            let current = offsets[1];
            if previous > current {
                return Err(InfluenceRowsError::OffsetsNotMonotonic {
                    index: previous_index + 1,
                    previous,
                    current,
                });
            }
        }

        let final_offset = vertex_offsets.last().copied().unwrap_or_default();
        if final_offset != influence_count {
            return Err(InfluenceRowsError::FinalOffsetMismatch {
                expected: influence_count,
                actual: final_offset,
            });
        }

        let maximum_row_len = vertex_offsets
            .windows(2)
            .map(|window| (window[1] - window[0]) as usize)
            .max()
            .unwrap_or_default();
        let mut source_orders = HashSet::new();
        // A construction scope can charge this entry count plus a conservative
        // bucket allowance before allocator entry. Exact process RSS remains
        // an external scale-gate concern because std keeps bucket layout private.
        source_orders.try_reserve(maximum_row_len).map_err(|_| {
            InfluenceRowsError::SourceOrderScratchAllocationFailed {
                requested_entries: maximum_row_len,
            }
        })?;

        for (vertex, offsets) in vertex_offsets.windows(2).enumerate() {
            source_orders.clear();
            let start = offsets[0] as usize;
            let end = offsets[1] as usize;

            for (influence_index, influence) in influences[start..end].iter().enumerate() {
                let influence_index = start + influence_index;
                if !influence.weight.is_finite() {
                    return Err(InfluenceRowsError::NonFiniteWeight { influence_index });
                }
                if influence.weight < 0.0 {
                    return Err(InfluenceRowsError::NegativeWeight { influence_index });
                }
                if !source_orders.insert(influence.source_order) {
                    return Err(InfluenceRowsError::DuplicateSourceOrder {
                        vertex: vertex as u32,
                        source_order: influence.source_order,
                    });
                }
            }
        }

        let retained_cell_capacity = vertex_offsets
            .capacity()
            .checked_add(influences.capacity())
            .ok_or(DeformCollectionError::SizeOverflow {
                resource: DeformBuildResource::Influences,
                count: usize::MAX,
            })?;
        let retained_allocation_bytes = retained_capacity_bytes::<u32>(
            vertex_offsets.capacity(),
            DeformBuildResource::InfluenceOffsets,
        )?
        .checked_add(retained_capacity_bytes::<Influence>(
            influences.capacity(),
            DeformBuildResource::Influences,
        )?)
        .ok_or(DeformCollectionError::SizeOverflow {
            resource: DeformBuildResource::Influences,
            count: retained_cell_capacity,
        })?;
        enforce_retained_bytes(
            DeformBuildResource::Influences,
            retained_allocation_bytes,
            MAX_DEFORM_COLLECTION_BYTES,
        )?;

        Ok(Self {
            vertex_count,
            vertex_offsets,
            influences,
            retained_cell_capacity,
            retained_allocation_bytes,
        })
    }

    pub const fn vertex_count(&self) -> u32 {
        self.vertex_count
    }

    pub fn vertex_offsets(&self) -> &[u32] {
        &self.vertex_offsets
    }

    pub fn influences(&self) -> &[Influence] {
        &self.influences
    }

    pub const fn retained_cell_capacity(&self) -> usize {
        self.retained_cell_capacity
    }

    pub const fn retained_allocation_bytes(&self) -> usize {
        self.retained_allocation_bytes
    }

    /// Returns one vertex row, or `None` when `vertex` is out of range.
    pub fn row(&self, vertex: u32) -> Option<&[Influence]> {
        if vertex >= self.vertex_count {
            return None;
        }

        let offset_index = vertex as usize;
        let start = *self.vertex_offsets.get(offset_index)? as usize;
        let end = *self.vertex_offsets.get(offset_index.checked_add(1)?)? as usize;
        self.influences.get(start..end)
    }
}

/// A structural or allocation failure while constructing influence rows.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum InfluenceRowsError {
    #[error(transparent)]
    Collection(#[from] DeformCollectionError),

    #[error("vertex count {vertex_count} cannot be represented with a terminal offset")]
    VertexCountOverflow { vertex_count: u32 },

    #[error("influence count {influence_count} exceeds the u32 offset domain")]
    InfluenceCountOverflow { influence_count: usize },

    #[error("influence offset count mismatch: expected {expected}, found {actual}")]
    OffsetCountMismatch { expected: usize, actual: usize },

    #[error("the first influence offset must be zero, found {actual}")]
    FirstOffsetNotZero { actual: u32 },

    #[error(
        "influence offset {index} is {offset}, beyond the flat influence count {influence_count}"
    )]
    OffsetOutOfRange {
        index: usize,
        offset: u32,
        influence_count: u32,
    },

    #[error(
        "influence offsets must be non-decreasing, but offset {index} changed from {previous} to {current}"
    )]
    OffsetsNotMonotonic {
        index: usize,
        previous: u32,
        current: u32,
    },

    #[error("the final influence offset must be {expected}, found {actual}")]
    FinalOffsetMismatch { expected: u32, actual: u32 },

    #[error("influence {influence_index} has a non-finite weight")]
    NonFiniteWeight { influence_index: usize },

    #[error("influence {influence_index} has a negative weight")]
    NegativeWeight { influence_index: usize },

    #[error("vertex {vertex} repeats source order {source_order}")]
    DuplicateSourceOrder { vertex: u32, source_order: u32 },

    #[error(
        "could not reserve conservative source-order validation scratch for {requested_entries} entries"
    )]
    SourceOrderScratchAllocationFailed { requested_entries: usize },
}

/// A coordinate space participating in the canonical skin matrix equation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkinMatrixSpace {
    LogicalMeshLocalBind,
    JointLocalBind,
    JointLocal,
    Scene,
    MeshInstanceLocal,
}

/// A source-declared correction matrix with explicit input and output spaces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinMatrixCorrection {
    from: SkinMatrixSpace,
    to: SkinMatrixSpace,
    matrix: Mat4,
}

impl SkinMatrixCorrection {
    pub fn new(
        from: SkinMatrixSpace,
        to: SkinMatrixSpace,
        matrix: Mat4,
    ) -> Result<Self, SkinMatrixCorrectionError> {
        if !matrix.is_finite() {
            return Err(SkinMatrixCorrectionError::NonFiniteMatrix);
        }

        Ok(Self { from, to, matrix })
    }

    pub const fn from(self) -> SkinMatrixSpace {
        self.from
    }

    pub const fn to(self) -> SkinMatrixSpace {
        self.to
    }

    pub const fn matrix(self) -> Mat4 {
        self.matrix
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SkinMatrixCorrectionError {
    #[error("skin correction matrix contains a non-finite value")]
    NonFiniteMatrix,
}

/// The explicit canonical matrix-space contract for skin evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinMatrixSpaces {
    input_position: SkinMatrixSpace,
    inverse_bind_from: SkinMatrixSpace,
    inverse_bind_to: SkinMatrixSpace,
    current_joint_from: SkinMatrixSpace,
    current_joint_to: SkinMatrixSpace,
    output_position: SkinMatrixSpace,
    correction: Option<SkinMatrixCorrection>,
}

impl SkinMatrixSpaces {
    /// Returns Baozi's canonical instance-local skinning contract.
    pub const fn canonical() -> Self {
        Self {
            input_position: SkinMatrixSpace::LogicalMeshLocalBind,
            inverse_bind_from: SkinMatrixSpace::LogicalMeshLocalBind,
            inverse_bind_to: SkinMatrixSpace::JointLocalBind,
            current_joint_from: SkinMatrixSpace::JointLocal,
            current_joint_to: SkinMatrixSpace::Scene,
            output_position: SkinMatrixSpace::MeshInstanceLocal,
            correction: None,
        }
    }

    /// Attaches a validated, source-declared correction without changing the
    /// canonical interpretation of the other matrices.
    pub const fn with_correction(mut self, correction: SkinMatrixCorrection) -> Self {
        self.correction = Some(correction);
        self
    }

    pub const fn input_position(self) -> SkinMatrixSpace {
        self.input_position
    }

    pub const fn inverse_bind_from(self) -> SkinMatrixSpace {
        self.inverse_bind_from
    }

    pub const fn inverse_bind_to(self) -> SkinMatrixSpace {
        self.inverse_bind_to
    }

    pub const fn current_joint_from(self) -> SkinMatrixSpace {
        self.current_joint_from
    }

    pub const fn current_joint_to(self) -> SkinMatrixSpace {
        self.current_joint_to
    }

    pub const fn output_position(self) -> SkinMatrixSpace {
        self.output_position
    }

    pub const fn correction(self) -> Option<SkinMatrixCorrection> {
        self.correction
    }
}

impl Default for SkinMatrixSpaces {
    fn default() -> Self {
        Self::canonical()
    }
}

/// An ordered joint palette and its source-preserving bind contract.
#[derive(PartialEq)]
pub struct Skin {
    name: Option<EntityName>,
    joints: Vec<NodeIndex>,
    inverse_bind_matrices: Vec<Mat4>,
    skeleton_root_hint: Option<NodeIndex>,
    matrix_spaces: SkinMatrixSpaces,
    provenance: Provenance,
    retained_allocation_bytes: usize,
}

impl std::fmt::Debug for Skin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Skin")
            .field("name", &self.name)
            .field("joint_count", &self.joints.len())
            .field(
                "inverse_bind_matrix_count",
                &self.inverse_bind_matrices.len(),
            )
            .field("has_skeleton_root_hint", &self.skeleton_root_hint.is_some())
            .field("matrix_spaces", &self.matrix_spaces)
            .field("provenance", &self.provenance)
            .finish()
    }
}

impl Skin {
    pub(crate) fn try_new<Joints, InverseBinds>(
        name: Option<EntityName>,
        joints: Joints,
        inverse_bind_matrices: InverseBinds,
        skeleton_root_hint: Option<NodeIndex>,
        matrix_spaces: SkinMatrixSpaces,
        provenance: Provenance,
    ) -> Result<Self, SkinError>
    where
        Joints: IntoIterator<Item = NodeIndex>,
        InverseBinds: IntoIterator<Item = Mat4>,
    {
        let joints = try_collect_deform(
            joints,
            DeformBuildResource::SkinJoints,
            MAX_SKIN_JOINTS,
            MAX_DEFORM_COLLECTION_BYTES,
        )?;
        let inverse_bind_matrices = try_collect_deform(
            inverse_bind_matrices,
            DeformBuildResource::SkinInverseBindMatrices,
            MAX_SKIN_JOINTS,
            MAX_DEFORM_COLLECTION_BYTES,
        )?;
        if joints.len() != inverse_bind_matrices.len() {
            return Err(SkinError::InverseBindCountMismatch {
                joints: joints.len(),
                inverse_bind_matrices: inverse_bind_matrices.len(),
            });
        }

        if let Some(joint) = inverse_bind_matrices
            .iter()
            .position(|matrix| !matrix.is_finite())
        {
            return Err(SkinError::NonFiniteInverseBindMatrix { joint });
        }

        let mut retained_allocation_bytes = retained_capacity_bytes::<NodeIndex>(
            joints.capacity(),
            DeformBuildResource::SkinJoints,
        )?
        .checked_add(retained_capacity_bytes::<Mat4>(
            inverse_bind_matrices.capacity(),
            DeformBuildResource::SkinInverseBindMatrices,
        )?)
        .ok_or(DeformCollectionError::SizeOverflow {
            resource: DeformBuildResource::SkinJoints,
            count: joints.len(),
        })?;
        if let Some(name) = &name {
            retained_allocation_bytes = retained_allocation_bytes
                .checked_add(name.capacity())
                .ok_or(DeformCollectionError::SizeOverflow {
                    resource: DeformBuildResource::SkinJoints,
                    count: joints.len(),
                })?;
        }
        enforce_retained_bytes(
            DeformBuildResource::SkinJoints,
            retained_allocation_bytes,
            MAX_DEFORM_COLLECTION_BYTES,
        )?;

        Ok(Self {
            name,
            joints,
            inverse_bind_matrices,
            skeleton_root_hint,
            matrix_spaces,
            provenance,
            retained_allocation_bytes,
        })
    }

    pub const fn name(&self) -> Option<&EntityName> {
        self.name.as_ref()
    }

    pub(crate) fn joints(&self) -> &[NodeIndex] {
        &self.joints
    }

    pub fn inverse_bind_matrices(&self) -> &[Mat4] {
        &self.inverse_bind_matrices
    }

    pub(crate) const fn skeleton_root_hint(&self) -> Option<NodeIndex> {
        self.skeleton_root_hint
    }

    pub const fn matrix_spaces(&self) -> SkinMatrixSpaces {
        self.matrix_spaces
    }

    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }

    pub const fn retained_allocation_bytes(&self) -> usize {
        self.retained_allocation_bytes
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SkinError {
    #[error(transparent)]
    Collection(#[from] DeformCollectionError),

    #[error(
        "skin joint and inverse-bind counts differ: {joints} joints, {inverse_bind_matrices} matrices"
    )]
    InverseBindCountMismatch {
        joints: usize,
        inverse_bind_matrices: usize,
    },

    #[error("inverse-bind matrix for joint {joint} contains a non-finite value")]
    NonFiniteInverseBindMatrix { joint: usize },
}

/// Logical-mesh-owned identity and source description for one morph target.
#[derive(Debug, PartialEq, Eq)]
pub struct MorphTargetDefinition {
    name: Option<EntityName>,
    provenance: Provenance,
    retained_allocation_bytes: usize,
}

impl MorphTargetDefinition {
    pub fn try_new(
        name: Option<EntityName>,
        provenance: Provenance,
    ) -> Result<Self, MorphTargetDefinitionError> {
        let retained_allocation_bytes = name.as_ref().map_or(0, EntityName::capacity);
        enforce_retained_bytes(
            DeformBuildResource::PrimitiveMorphTarget,
            retained_allocation_bytes,
            MAX_DEFORM_COLLECTION_BYTES,
        )?;
        Ok(Self {
            name,
            provenance,
            retained_allocation_bytes,
        })
    }

    pub const fn name(&self) -> Option<&EntityName> {
        self.name.as_ref()
    }

    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }

    pub const fn retained_allocation_bytes(&self) -> usize {
        self.retained_allocation_bytes
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MorphTargetDefinitionError {
    #[error(transparent)]
    Collection(#[from] DeformCollectionError),
}

/// A currently admitted relative morph-delta semantic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MorphDeltaSemantic {
    Position,
    Normal,
    Tangent,
}

/// One finite, relative F32x3 morph stream and its source attachment.
#[derive(Debug, PartialEq)]
pub struct MorphDelta {
    payload: AttributePayload,
    provenance: Provenance,
    retained_cell_capacity: usize,
    retained_allocation_bytes: usize,
}

impl MorphDelta {
    pub fn try_new(
        payload: AttributePayload,
        provenance: Provenance,
    ) -> Result<Self, MorphDeltaError> {
        let format = payload
            .format()
            .map_err(|cause| MorphDeltaError::MalformedPayload { cause })?;
        if !format.is_plain_f32(3) {
            return Err(MorphDeltaError::InvalidFormat);
        }

        if let Some(default) = payload.default_value()
            && let Some((element, lane)) =
                first_f32x3_value_matching(default, |value| value != 0.0)?
        {
            return Err(MorphDeltaError::NonZeroSparseDefault { element, lane });
        }
        let retained_allocation_bytes = payload.retained_allocation_bytes();
        enforce_retained_bytes(
            DeformBuildResource::MorphDelta,
            retained_allocation_bytes,
            MAX_MORPH_DELTA_RETAINED_BYTES,
        )?;
        let cell_width = std::mem::size_of::<f32>();
        if !retained_allocation_bytes.is_multiple_of(cell_width) {
            return Err(MorphDeltaError::InvalidRetainedLayout);
        }

        Ok(Self {
            payload,
            provenance,
            retained_cell_capacity: retained_allocation_bytes / cell_width,
            retained_allocation_bytes,
        })
    }

    pub const fn payload(&self) -> &AttributePayload {
        &self.payload
    }

    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }

    pub const fn retained_cell_capacity(&self) -> usize {
        self.retained_cell_capacity
    }

    pub const fn retained_allocation_bytes(&self) -> usize {
        self.retained_allocation_bytes
    }
}

/// A rejected typed morph delta payload.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MorphDeltaError {
    #[error(transparent)]
    Collection(#[from] DeformCollectionError),

    #[error("morph delta payload is internally malformed: {cause}")]
    MalformedPayload {
        #[source]
        cause: AttributePayloadError,
    },

    #[error("morph delta must use plain F32x3 values")]
    InvalidFormat,

    #[error("morph delta uses storage incompatible with plain F32x3 values")]
    InvalidTypedStorage,

    #[error("morph delta retained storage is not aligned to canonical scalar cells")]
    InvalidRetainedLayout,

    #[error("morph delta sparse default element {element} lane {lane} is not zero")]
    NonZeroSparseDefault { element: usize, lane: u8 },
}

fn first_f32x3_value_matching(
    buffer: &AttributeBuffer,
    mut predicate: impl FnMut(f32) -> bool,
) -> Result<Option<(usize, u8)>, MorphDeltaError> {
    let values = buffer
        .f32_vec3_view()
        .ok_or(MorphDeltaError::InvalidTypedStorage)?;
    for (element, value) in values.iter().enumerate() {
        for (lane, component) in [0_u8, 1, 2].into_iter().zip([value.x, value.y, value.z]) {
            if predicate(component) {
                return Ok(Some((element, lane)));
            }
        }
    }
    Ok(None)
}

/// Primitive-local morph delta payloads for one logical target identity.
#[derive(PartialEq)]
pub struct PrimitiveMorphTarget {
    target: MorphTargetLocalIndex,
    vertex_count: u32,
    position: Option<MorphDelta>,
    normal: Option<MorphDelta>,
    tangent: Option<MorphDelta>,
    retained_cell_capacity: usize,
    retained_allocation_bytes: usize,
}

impl std::fmt::Debug for PrimitiveMorphTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PrimitiveMorphTarget")
            .field("target", &"opaque")
            .field("vertex_count", &self.vertex_count)
            .field("has_position_delta", &self.position.is_some())
            .field("has_normal_delta", &self.normal.is_some())
            .field("has_tangent_delta", &self.tangent.is_some())
            .finish()
    }
}

impl PrimitiveMorphTarget {
    pub(crate) fn try_new(
        target: MorphTargetLocalIndex,
        vertex_count: u32,
        deltas: impl IntoIterator<Item = (MorphDeltaSemantic, MorphDelta)>,
    ) -> Result<Self, PrimitiveMorphTargetError> {
        let mut result = Self {
            target,
            vertex_count,
            position: None,
            normal: None,
            tangent: None,
            retained_cell_capacity: 0,
            retained_allocation_bytes: 0,
        };

        for (semantic, delta) in deltas {
            let actual = delta.payload.logical_count();
            if actual != vertex_count {
                return Err(PrimitiveMorphTargetError::VertexCountMismatch {
                    semantic,
                    expected: vertex_count,
                    actual,
                });
            }

            let duplicate = match semantic {
                MorphDeltaSemantic::Position => result.position.is_some(),
                MorphDeltaSemantic::Normal => result.normal.is_some(),
                MorphDeltaSemantic::Tangent => result.tangent.is_some(),
            };
            if duplicate {
                return Err(PrimitiveMorphTargetError::DuplicateSemantic { semantic });
            }
            let retained_cell_capacity = result
                .retained_cell_capacity
                .checked_add(delta.retained_cell_capacity())
                .ok_or(DeformCollectionError::SizeOverflow {
                    resource: DeformBuildResource::PrimitiveMorphTarget,
                    count: usize::MAX,
                })?;
            let retained_allocation_bytes = result
                .retained_allocation_bytes
                .checked_add(delta.retained_allocation_bytes())
                .ok_or(DeformCollectionError::SizeOverflow {
                    resource: DeformBuildResource::PrimitiveMorphTarget,
                    count: retained_cell_capacity,
                })?;
            enforce_retained_bytes(
                DeformBuildResource::PrimitiveMorphTarget,
                retained_allocation_bytes,
                MAX_MORPH_DELTA_RETAINED_BYTES,
            )?;
            match semantic {
                MorphDeltaSemantic::Position => result.position = Some(delta),
                MorphDeltaSemantic::Normal => result.normal = Some(delta),
                MorphDeltaSemantic::Tangent => result.tangent = Some(delta),
            }
            result.retained_cell_capacity = retained_cell_capacity;
            result.retained_allocation_bytes = retained_allocation_bytes;
        }

        Ok(result)
    }

    pub(crate) const fn target(&self) -> MorphTargetLocalIndex {
        self.target
    }

    pub const fn vertex_count(&self) -> u32 {
        self.vertex_count
    }

    pub fn delta(&self, semantic: MorphDeltaSemantic) -> Option<&MorphDelta> {
        match semantic {
            MorphDeltaSemantic::Position => self.position.as_ref(),
            MorphDeltaSemantic::Normal => self.normal.as_ref(),
            MorphDeltaSemantic::Tangent => self.tangent.as_ref(),
        }
    }

    pub const fn retained_cell_capacity(&self) -> usize {
        self.retained_cell_capacity
    }

    pub const fn retained_allocation_bytes(&self) -> usize {
        self.retained_allocation_bytes
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveMorphTargetError {
    #[error(transparent)]
    Collection(#[from] DeformCollectionError),

    #[error("{semantic:?} morph delta has logical vertex count {actual}, expected {expected}")]
    VertexCountMismatch {
        semantic: MorphDeltaSemantic,
        expected: u32,
        actual: u32,
    },

    #[error("primitive morph target repeats the {semantic:?} delta semantic")]
    DuplicateSemantic { semantic: MorphDeltaSemantic },
}

/// Validates a complete per-instance override of logical-mesh morph weights.
pub fn validate_morph_weight_override(
    target_count: usize,
    weights: &[f32],
) -> Result<(), MorphWeightOverrideError> {
    if weights.len() != target_count {
        return Err(MorphWeightOverrideError::CountMismatch {
            expected: target_count,
            actual: weights.len(),
        });
    }

    if let Some(index) = weights.iter().position(|weight| !weight.is_finite()) {
        return Err(MorphWeightOverrideError::NonFiniteWeight { index });
    }

    Ok(())
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MorphWeightOverrideError {
    #[error("morph override count mismatch: expected {expected}, found {actual}")]
    CountMismatch { expected: usize, actual: usize },

    #[error("morph override weight {index} is not finite")]
    NonFiniteWeight { index: usize },
}

fn preflight_deform_collection<T>(
    resource: DeformBuildResource,
    count: usize,
    maximum_count: usize,
    maximum_bytes: usize,
) -> Result<usize, DeformCollectionError> {
    if count > maximum_count {
        return Err(DeformCollectionError::LimitExceeded {
            resource,
            dimension: DeformLimitDimension::Count,
            requested: count,
            maximum: maximum_count,
        });
    }
    let bytes = count
        .checked_mul(std::mem::size_of::<T>())
        .ok_or(DeformCollectionError::SizeOverflow { resource, count })?;
    enforce_retained_bytes(resource, bytes, maximum_bytes)?;
    Ok(bytes)
}

fn retained_capacity_bytes<T>(
    capacity: usize,
    resource: DeformBuildResource,
) -> Result<usize, DeformCollectionError> {
    capacity
        .checked_mul(std::mem::size_of::<T>())
        .ok_or(DeformCollectionError::SizeOverflow {
            resource,
            count: capacity,
        })
}

fn enforce_retained_bytes(
    resource: DeformBuildResource,
    requested: usize,
    maximum: usize,
) -> Result<(), DeformCollectionError> {
    if requested <= maximum {
        Ok(())
    } else {
        Err(DeformCollectionError::LimitExceeded {
            resource,
            dimension: DeformLimitDimension::Bytes,
            requested,
            maximum,
        })
    }
}

fn try_collect_deform<T>(
    values: impl IntoIterator<Item = T>,
    resource: DeformBuildResource,
    maximum_count: usize,
    maximum_bytes: usize,
) -> Result<Vec<T>, DeformCollectionError> {
    let mut iterator = values.into_iter();
    let mut collected = Vec::new();
    let (minimum, _) = iterator.size_hint();
    if minimum != 0 {
        try_reserve_deform_collection(
            &mut collected,
            minimum,
            resource,
            maximum_count,
            maximum_bytes,
        )?;
    }

    for value in &mut iterator {
        let desired =
            collected
                .len()
                .checked_add(1)
                .ok_or(DeformCollectionError::SizeOverflow {
                    resource,
                    count: usize::MAX,
                })?;
        preflight_deform_collection::<T>(resource, desired, maximum_count, maximum_bytes)?;
        if collected.len() == collected.capacity() {
            try_reserve_deform_collection(
                &mut collected,
                desired,
                resource,
                maximum_count,
                maximum_bytes,
            )?;
        }
        collected.push(value);
    }
    Ok(collected)
}

fn try_reserve_deform_collection<T>(
    values: &mut Vec<T>,
    desired: usize,
    resource: DeformBuildResource,
    maximum_count: usize,
    maximum_bytes: usize,
) -> Result<(), DeformCollectionError> {
    preflight_deform_collection::<T>(resource, desired, maximum_count, maximum_bytes)?;
    if desired <= values.capacity() {
        return Ok(());
    }

    let plan = CapacityPlan::for_type::<T>(values.len(), values.capacity(), desired)
        .map_err(|_| DeformCollectionError::Allocation { resource })?;
    preflight_deform_collection::<T>(
        resource,
        plan.envelope_capacity(),
        maximum_count,
        maximum_bytes,
    )?;
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| DeformCollectionError::Allocation { resource })?;
    plan.finish(values.len(), values.capacity())
        .map_err(|_| DeformCollectionError::Allocation { resource })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attribute::{
        AttributeBuffer, AttributeBufferError, AttributeFormat, AttributePayload, ComponentType,
        Normalization,
    };
    use crate::entity::{EntityName, EntityNameError, MAX_ENTITY_NAME_BYTES};
    use crate::identity::{MorphTargetLocalIndex, NodeIndex};
    use crate::math::{Vec3, Vec4};
    use crate::provenance::test_provenance_fixture;
    use std::marker::PhantomData;

    struct SizeHintOnly<T> {
        minimum: usize,
        marker: PhantomData<T>,
    }

    impl<T> SizeHintOnly<T> {
        const fn new(minimum: usize) -> Self {
            Self {
                minimum,
                marker: PhantomData,
            }
        }
    }

    impl<T> Iterator for SizeHintOnly<T> {
        type Item = T;

        fn next(&mut self) -> Option<Self::Item> {
            None
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            (self.minimum, None)
        }
    }

    fn morph_target_index(local: u32) -> MorphTargetLocalIndex {
        MorphTargetLocalIndex::from_u32(local)
    }

    fn node_index(index: u32) -> NodeIndex {
        NodeIndex::from_u32(index)
    }

    fn f32x3_format() -> AttributeFormat {
        AttributeFormat::new(ComponentType::F32, 3, Normalization::None, None).unwrap()
    }

    fn flat_f32x3(values: Vec<f32>) -> AttributeBuffer {
        AttributeBuffer::from_f32(f32x3_format(), values).unwrap()
    }

    fn influences(count: u32) -> Vec<Influence> {
        (0..count)
            .map(|source_order| Influence::new(source_order, 1.0, source_order))
            .collect()
    }

    #[test]
    fn preserves_zero_four_and_over_four_influence_rows() {
        let mut flat = influences(4);
        flat.extend(influences(6));
        let rows = InfluenceRows::try_new(3, vec![0, 0, 4, 10], flat).unwrap();

        assert_eq!(rows.vertex_count(), 3);
        assert_eq!(rows.row(0).unwrap().len(), 0);
        assert_eq!(rows.row(1).unwrap().len(), 4);
        assert_eq!(rows.row(2).unwrap().len(), 6);
        assert!(rows.row(3).is_none());
        assert_eq!(rows.influences()[9].palette_joint(), 5);
    }

    #[test]
    fn accepts_empty_vertex_domain_with_only_the_terminal_offset() {
        let rows = InfluenceRows::try_new(0, vec![0], Vec::new()).unwrap();

        assert_eq!(rows.vertex_count(), 0);
        assert_eq!(rows.vertex_offsets(), &[0]);
        assert!(rows.influences().is_empty());
    }

    #[test]
    fn rejects_influence_offset_contract_violations() {
        assert_eq!(
            InfluenceRows::try_new(1, vec![0], Vec::new()),
            Err(InfluenceRowsError::OffsetCountMismatch {
                expected: 2,
                actual: 1,
            })
        );
        assert_eq!(
            InfluenceRows::try_new(1, vec![1, 1], influences(1)),
            Err(InfluenceRowsError::FirstOffsetNotZero { actual: 1 })
        );
        assert_eq!(
            InfluenceRows::try_new(2, vec![0, 2, 1], influences(2)),
            Err(InfluenceRowsError::OffsetsNotMonotonic {
                index: 2,
                previous: 2,
                current: 1,
            })
        );
        assert_eq!(
            InfluenceRows::try_new(1, vec![0, 2], influences(1)),
            Err(InfluenceRowsError::OffsetOutOfRange {
                index: 1,
                offset: 2,
                influence_count: 1,
            })
        );
        assert_eq!(
            InfluenceRows::try_new(1, vec![0, 0], influences(1)),
            Err(InfluenceRowsError::FinalOffsetMismatch {
                expected: 1,
                actual: 0,
            })
        );
        assert_eq!(
            InfluenceRows::try_new(u32::MAX, Vec::new(), Vec::new()),
            Err(InfluenceRowsError::VertexCountOverflow {
                vertex_count: u32::MAX,
            })
        );
    }

    #[test]
    fn rejects_invalid_weights_and_duplicate_source_order_per_row() {
        assert_eq!(
            InfluenceRows::try_new(1, vec![0, 1], vec![Influence::new(0, -0.5, 0)]),
            Err(InfluenceRowsError::NegativeWeight { influence_index: 0 })
        );
        assert_eq!(
            InfluenceRows::try_new(1, vec![0, 1], vec![Influence::new(0, f32::NAN, 0)]),
            Err(InfluenceRowsError::NonFiniteWeight { influence_index: 0 })
        );
        assert_eq!(
            InfluenceRows::try_new(
                1,
                vec![0, 2],
                vec![Influence::new(0, 0.5, 7), Influence::new(1, 0.5, 7)],
            ),
            Err(InfluenceRowsError::DuplicateSourceOrder {
                vertex: 0,
                source_order: 7,
            })
        );

        assert!(
            InfluenceRows::try_new(
                2,
                vec![0, 1, 2],
                vec![Influence::new(0, 0.0, 3), Influence::new(0, -0.0, 3)],
            )
            .is_ok()
        );
    }

    #[test]
    fn canonical_skin_spaces_and_typed_correction_are_explicit() {
        let correction = SkinMatrixCorrection::new(
            SkinMatrixSpace::LogicalMeshLocalBind,
            SkinMatrixSpace::MeshInstanceLocal,
            Mat4::IDENTITY,
        )
        .unwrap();
        let spaces = SkinMatrixSpaces::canonical().with_correction(correction);

        assert_eq!(
            spaces.input_position(),
            SkinMatrixSpace::LogicalMeshLocalBind
        );
        assert_eq!(spaces.inverse_bind_to(), SkinMatrixSpace::JointLocalBind);
        assert_eq!(spaces.current_joint_from(), SkinMatrixSpace::JointLocal);
        assert_eq!(spaces.current_joint_to(), SkinMatrixSpace::Scene);
        assert_eq!(spaces.output_position(), SkinMatrixSpace::MeshInstanceLocal);
        assert_eq!(spaces.correction(), Some(correction));

        let mut non_finite = Mat4::IDENTITY;
        non_finite.cols[2][1] = f32::INFINITY;
        assert_eq!(
            SkinMatrixCorrection::new(
                SkinMatrixSpace::LogicalMeshLocalBind,
                SkinMatrixSpace::Scene,
                non_finite,
            ),
            Err(SkinMatrixCorrectionError::NonFiniteMatrix)
        );
    }

    #[test]
    fn skin_requires_exactly_one_finite_inverse_bind_per_joint() {
        let (_arena, provenance) = test_provenance_fixture();
        let joint = node_index(0);
        let mismatch = Skin::try_new(
            Some(EntityName::try_new("armature").expect("test name should be valid")),
            vec![joint],
            Vec::new(),
            Some(joint),
            SkinMatrixSpaces::canonical(),
            provenance,
        );
        assert_eq!(
            mismatch,
            Err(SkinError::InverseBindCountMismatch {
                joints: 1,
                inverse_bind_matrices: 0,
            })
        );

        let mut non_finite = Mat4::IDENTITY;
        non_finite.cols[0][0] = f32::NAN;
        assert_eq!(
            Skin::try_new(
                None,
                vec![joint],
                vec![non_finite],
                None,
                SkinMatrixSpaces::canonical(),
                provenance,
            ),
            Err(SkinError::NonFiniteInverseBindMatrix { joint: 0 })
        );

        let skin = Skin::try_new(
            Some(EntityName::try_new("armature").expect("test name should be valid")),
            vec![joint],
            vec![Mat4::IDENTITY],
            Some(joint),
            SkinMatrixSpaces::canonical(),
            provenance,
        )
        .unwrap();
        assert_eq!(skin.name().map(EntityName::as_str), Some("armature"));
        assert_eq!(skin.joints(), &[joint]);
        assert_eq!(skin.skeleton_root_hint(), Some(joint));
        assert_eq!(skin.provenance(), provenance);
    }

    #[test]
    fn public_debug_output_hides_storage_indices() {
        let (_arena, provenance) = test_provenance_fixture();
        let private_index = 4_294_967_291;
        let skin = Skin::try_new(
            None,
            vec![node_index(private_index)],
            vec![Mat4::IDENTITY],
            Some(node_index(private_index)),
            SkinMatrixSpaces::canonical(),
            provenance,
        )
        .unwrap();
        let target =
            PrimitiveMorphTarget::try_new(morph_target_index(private_index), 0, std::iter::empty())
                .unwrap();

        let skin_debug = format!("{skin:?}");
        let target_debug = format!("{target:?}");
        assert!(!skin_debug.contains(&private_index.to_string()));
        assert!(!target_debug.contains(&private_index.to_string()));
        assert!(skin_debug.contains("joint_count: 1"));
        assert!(target_debug.contains("target: \"opaque\""));
    }

    #[test]
    fn accepts_dense_and_sparse_f32x3_morph_deltas() {
        let (_arena, provenance) = test_provenance_fixture();
        let dense = MorphDelta::try_new(
            AttributePayload::dense(
                AttributeBuffer::from_f32_vec3(vec![Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)]).unwrap(),
            ),
            provenance,
        )
        .unwrap();
        let sparse = MorphDelta::try_new(
            AttributePayload::sparse(
                2,
                vec![1],
                AttributeBuffer::from_f32_vec3(vec![Vec3::new(0.0, 1.0, 0.0)]).unwrap(),
                AttributeBuffer::from_f32_vec3(vec![Vec3::ZERO]).unwrap(),
            )
            .unwrap(),
            provenance,
        )
        .unwrap();
        let target = PrimitiveMorphTarget::try_new(
            morph_target_index(0),
            2,
            [
                (MorphDeltaSemantic::Position, dense),
                (MorphDeltaSemantic::Tangent, sparse),
            ],
        )
        .unwrap();

        assert!(
            !target
                .delta(MorphDeltaSemantic::Position)
                .unwrap()
                .payload()
                .is_sparse()
        );
        assert!(
            target
                .delta(MorphDeltaSemantic::Tangent)
                .unwrap()
                .payload()
                .is_sparse()
        );
        assert!(target.delta(MorphDeltaSemantic::Normal).is_none());
        assert_eq!(
            target
                .delta(MorphDeltaSemantic::Position)
                .unwrap()
                .provenance(),
            provenance
        );
        assert_eq!(target.target(), morph_target_index(0));
        assert_eq!(target.vertex_count(), 2);
    }

    #[test]
    fn accepts_flat_and_specialized_f32x3_typed_storage() {
        let (_arena, provenance) = test_provenance_fixture();
        let flat = MorphDelta::try_new(
            AttributePayload::dense(flat_f32x3(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0])),
            provenance,
        )
        .unwrap();
        let specialized = MorphDelta::try_new(
            AttributePayload::dense(
                AttributeBuffer::from_f32_vec3(vec![Vec3::new(1.0, 2.0, 3.0)]).unwrap(),
            ),
            provenance,
        )
        .unwrap();

        assert_eq!(flat.payload().logical_count(), 2);
        assert_eq!(specialized.payload().logical_count(), 1);
    }

    #[test]
    fn maps_malformed_attribute_payload_before_morph_rules() {
        let (_arena, provenance) = test_provenance_fixture();
        let dense_nan = AttributePayload::dense(AttributeBuffer::from_f32_unchecked_for_test(
            f32x3_format(),
            vec![0.0, f32::NAN, 0.0],
        ));
        assert_eq!(
            MorphDelta::try_new(dense_nan, provenance),
            Err(MorphDeltaError::MalformedPayload {
                cause: AttributePayloadError::Buffer(AttributeBufferError::NonFiniteValue {
                    element_index: 1,
                }),
            })
        );
    }

    #[test]
    fn rejects_nonzero_sparse_morph_defaults() {
        let (_arena, provenance) = test_provenance_fixture();
        let sparse = AttributePayload::sparse(
            3,
            vec![2],
            flat_f32x3(vec![1.0, 0.0, 0.0]),
            flat_f32x3(vec![0.0, -0.25, 0.0]),
        )
        .unwrap();

        assert_eq!(
            MorphDelta::try_new(sparse, provenance),
            Err(MorphDeltaError::NonZeroSparseDefault {
                element: 0,
                lane: 1,
            })
        );
    }

    #[test]
    fn rejects_wrong_morph_cardinality_format_and_duplicate_semantic() {
        let (_arena, provenance) = test_provenance_fixture();
        let wrong_count = MorphDelta::try_new(
            AttributePayload::dense(AttributeBuffer::from_f32_vec3(vec![Vec3::ZERO]).unwrap()),
            provenance,
        )
        .unwrap();
        assert_eq!(
            PrimitiveMorphTarget::try_new(
                morph_target_index(0),
                2,
                [(MorphDeltaSemantic::Position, wrong_count)],
            ),
            Err(PrimitiveMorphTargetError::VertexCountMismatch {
                semantic: MorphDeltaSemantic::Position,
                expected: 2,
                actual: 1,
            })
        );

        assert_eq!(
            MorphDelta::try_new(
                AttributePayload::dense(
                    AttributeBuffer::from_f32_vec4(vec![Vec4::new(0.0, 0.0, 0.0, 1.0)]).unwrap(),
                ),
                provenance,
            ),
            Err(MorphDeltaError::InvalidFormat)
        );

        let first = MorphDelta::try_new(
            AttributePayload::dense(AttributeBuffer::from_f32_vec3(vec![Vec3::ZERO]).unwrap()),
            provenance,
        )
        .unwrap();
        let second = MorphDelta::try_new(
            AttributePayload::dense(AttributeBuffer::from_f32_vec3(vec![Vec3::ZERO]).unwrap()),
            provenance,
        )
        .unwrap();
        assert_eq!(
            PrimitiveMorphTarget::try_new(
                morph_target_index(0),
                1,
                [
                    (MorphDeltaSemantic::Normal, first),
                    (MorphDeltaSemantic::Normal, second),
                ],
            ),
            Err(PrimitiveMorphTargetError::DuplicateSemantic {
                semantic: MorphDeltaSemantic::Normal,
            })
        );
    }

    #[test]
    fn morph_definitions_retain_name_and_provenance() {
        let (_arena, provenance) = test_provenance_fixture();
        let definition = MorphTargetDefinition::try_new(
            Some(EntityName::try_new("smile").expect("test name should be valid")),
            provenance,
        )
        .expect("bounded morph definition should be admitted");

        assert_eq!(definition.name().map(EntityName::as_str), Some("smile"));
        assert_eq!(definition.provenance(), provenance);
    }

    #[test]
    fn deform_construction_discards_hostile_capacity_and_reports_retained_storage() {
        let mut offsets = Vec::with_capacity(1_024);
        offsets.extend([0, 1]);
        let mut flat = Vec::with_capacity(1_024);
        flat.push(Influence::new(0, 1.0, 0));
        let source_row_bytes = offsets.capacity() * std::mem::size_of::<u32>()
            + flat.capacity() * std::mem::size_of::<Influence>();

        let rows = InfluenceRows::try_new(1, offsets, flat)
            .expect("one bounded influence row should be admitted");
        assert!(rows.retained_allocation_bytes() < source_row_bytes);

        let mut joints = Vec::with_capacity(1_024);
        joints.push(node_index(0));
        let mut inverse_binds = Vec::with_capacity(1_024);
        inverse_binds.push(Mat4::IDENTITY);
        let source_skin_bytes = joints.capacity() * std::mem::size_of::<NodeIndex>()
            + inverse_binds.capacity() * std::mem::size_of::<Mat4>();
        let skin = Skin::try_new(
            Some(EntityName::try_new("armature").expect("test name should be valid")),
            joints,
            inverse_binds,
            None,
            SkinMatrixSpaces::canonical(),
            test_provenance_fixture().1,
        )
        .expect("one bounded skin joint should be admitted");
        assert!(skin.retained_allocation_bytes() < source_skin_bytes);

        let mut dense_values = Vec::with_capacity(1_024);
        dense_values.push(Vec3::ZERO);
        let source_delta_bytes = dense_values.capacity() * std::mem::size_of::<Vec3>();
        let delta = MorphDelta::try_new(
            AttributePayload::dense(
                AttributeBuffer::from_f32_vec3(dense_values)
                    .expect("test morph buffer should be valid"),
            ),
            test_provenance_fixture().1,
        )
        .expect("one bounded morph delta should be admitted");
        assert_eq!(delta.retained_cell_capacity(), 3);
        assert!(delta.retained_allocation_bytes() < source_delta_bytes);
    }

    #[test]
    fn deform_collection_limits_reject_hostile_hints_before_allocator_entry() {
        assert!(matches!(
            InfluenceRows::try_new(
                0,
                SizeHintOnly::<u32>::new(MAX_INFLUENCE_VERTICES + 2),
                std::iter::empty(),
            ),
            Err(InfluenceRowsError::Collection(
                DeformCollectionError::LimitExceeded {
                    resource: DeformBuildResource::InfluenceOffsets,
                    dimension: DeformLimitDimension::Count,
                    requested,
                    maximum,
                }
            )) if requested == MAX_INFLUENCE_VERTICES + 2
                && maximum == MAX_INFLUENCE_VERTICES + 1
        ));

        let (_arena, provenance) = test_provenance_fixture();
        assert!(matches!(
            Skin::try_new(
                None,
                SizeHintOnly::<NodeIndex>::new(MAX_SKIN_JOINTS + 1),
                std::iter::empty(),
                None,
                SkinMatrixSpaces::canonical(),
                provenance,
            ),
            Err(SkinError::Collection(DeformCollectionError::LimitExceeded {
                resource: DeformBuildResource::SkinJoints,
                dimension: DeformLimitDimension::Count,
                requested,
                maximum: MAX_SKIN_JOINTS,
            })) if requested == MAX_SKIN_JOINTS + 1
        ));
    }

    #[test]
    fn deform_growth_rejects_capacity_envelope_before_allocator_entry() {
        let mut values = Vec::<u32>::new();

        assert_eq!(
            try_reserve_deform_collection(
                &mut values,
                3,
                DeformBuildResource::InfluenceOffsets,
                3,
                3 * std::mem::size_of::<u32>(),
            ),
            Err(DeformCollectionError::LimitExceeded {
                resource: DeformBuildResource::InfluenceOffsets,
                dimension: DeformLimitDimension::Count,
                requested: 4,
                maximum: 3,
            })
        );
        assert_eq!(values.capacity(), 0);
    }

    #[test]
    fn skin_and_morph_names_use_the_bounded_entity_name_contract() {
        let hostile = "x".repeat(MAX_ENTITY_NAME_BYTES + 1);
        assert_eq!(
            EntityName::try_new(&hostile),
            Err(EntityNameError::TooLong {
                actual: MAX_ENTITY_NAME_BYTES + 1,
                maximum: MAX_ENTITY_NAME_BYTES,
            })
        );

        let (_arena, provenance) = test_provenance_fixture();
        let definition = MorphTargetDefinition::try_new(
            Some(EntityName::try_new("smile").expect("test name should be valid")),
            provenance,
        )
        .expect("bounded morph definition should be admitted");
        assert_eq!(definition.name().map(EntityName::as_str), Some("smile"));
        assert!(definition.retained_allocation_bytes() >= "smile".len());
    }

    #[test]
    fn instance_morph_overrides_are_complete_and_finite() {
        assert_eq!(validate_morph_weight_override(0, &[]), Ok(()));
        assert_eq!(validate_morph_weight_override(3, &[0.0, -1.0, 2.5]), Ok(()));
        assert_eq!(
            validate_morph_weight_override(2, &[0.0]),
            Err(MorphWeightOverrideError::CountMismatch {
                expected: 2,
                actual: 1,
            })
        );
        assert_eq!(
            validate_morph_weight_override(2, &[0.0, f32::NEG_INFINITY]),
            Err(MorphWeightOverrideError::NonFiniteWeight { index: 1 })
        );
    }
}
