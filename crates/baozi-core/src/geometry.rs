use crate::attribute::{
    Attribute, AttributeDomain, AttributeKeyValidationError, AttributeSemantic,
    validate_unique_attribute_keys,
};
use crate::deform::{
    InfluenceRows, MorphDeltaSemantic, MorphTargetDefinition, MorphWeightOverrideError,
    PrimitiveMorphTarget, validate_morph_weight_override,
};
use crate::derived::{Derived, DerivedAbsenceReason};
use crate::entity::EntityName;
use crate::identity::{LogicalMeshIndex, MaterialIndex, NodeIndex, SkinIndex};
use crate::math::Aabb;
use crate::provenance::ProvenanceAttachments;
use baozi_operation::CapacityPlan;
use std::convert::Infallible;
use std::fmt;
use std::mem::size_of;
use thiserror::Error;

/// A geometry collection whose position is represented by a canonical `u32` index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GeometryCollection {
    IndexElements,
    PolygonFaces,
    PrimitiveMorphTargets,
    LogicalMeshPrimitives,
    LogicalMeshMorphTargets,
}

/// One heap-backed collection owned by canonical geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GeometryAllocation {
    PolygonFaces,
    IndexElements,
    PrimitiveAttributes,
    PrimitiveInfluences,
    PrimitiveMorphTargets,
    PrimitiveBoundsProducer,
    PrimitiveProvenanceAttachments,
    LogicalMeshName,
    LogicalMeshPrimitives,
    LogicalMeshMorphTargets,
    LogicalMeshDefaultMorphWeights,
    LogicalMeshBoundsProducer,
    LogicalMeshProvenanceAttachments,
    MeshInstanceMorphWeights,
    MeshInstanceAttributes,
    MeshInstanceProvenanceAttachments,
}

/// A failed fallible allocation or retained-capacity calculation.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum GeometryStorageError {
    #[error(transparent)]
    Count(#[from] GeometryCountError),

    #[error("geometry allocation failed for {resource:?}")]
    AllocationFailed { resource: GeometryAllocation },

    #[error("geometry collection size overflow for {resource:?}")]
    CollectionSizeOverflow { resource: GeometryAllocation },

    #[error("retained geometry footprint overflow for {resource:?}")]
    RetainedFootprintOverflow { resource: GeometryAllocation },
}

pub(crate) enum ControlledGeometryStorageError<E> {
    Storage(GeometryStorageError),
    Control(E),
}

impl<E> From<GeometryStorageError> for ControlledGeometryStorageError<E> {
    fn from(error: GeometryStorageError) -> Self {
        Self::Storage(error)
    }
}

fn unwrap_uncontrolled_footprint<T>(
    result: Result<T, ControlledGeometryStorageError<Infallible>>,
) -> Result<T, GeometryStorageError> {
    match result {
        Ok(value) => Ok(value),
        Err(ControlledGeometryStorageError::Storage(error)) => Err(error),
        Err(ControlledGeometryStorageError::Control(error)) => match error {},
    }
}

/// Allocator-visible capacity retained by one canonical geometry owner.
///
/// Collection slots count geometry-level `Vec` capacity plus deformation
/// capacity exposed by nested values. Bytes also include nested attribute,
/// influence, morph, and bounded-name allocations.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GeometryRetainedFootprint {
    collection_slots: usize,
    bytes: usize,
}

impl GeometryRetainedFootprint {
    pub const fn collection_slots(self) -> usize {
        self.collection_slots
    }

    pub const fn bytes(self) -> usize {
        self.bytes
    }

    fn try_add_capacity<T>(
        &mut self,
        capacity: usize,
        resource: GeometryAllocation,
    ) -> Result<(), GeometryStorageError> {
        let allocation_bytes = capacity
            .checked_mul(size_of::<T>())
            .ok_or(GeometryStorageError::RetainedFootprintOverflow { resource })?;
        let collection_slots = self
            .collection_slots
            .checked_add(capacity)
            .ok_or(GeometryStorageError::RetainedFootprintOverflow { resource })?;
        let bytes = self
            .bytes
            .checked_add(allocation_bytes)
            .ok_or(GeometryStorageError::RetainedFootprintOverflow { resource })?;
        self.collection_slots = collection_slots;
        self.bytes = bytes;
        Ok(())
    }

    fn try_add_nested(
        &mut self,
        collection_slots: usize,
        bytes: usize,
        resource: GeometryAllocation,
    ) -> Result<(), GeometryStorageError> {
        let next_slots = self
            .collection_slots
            .checked_add(collection_slots)
            .ok_or(GeometryStorageError::RetainedFootprintOverflow { resource })?;
        let next_bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or(GeometryStorageError::RetainedFootprintOverflow { resource })?;
        self.collection_slots = next_slots;
        self.bytes = next_bytes;
        Ok(())
    }

    fn try_add_bytes(
        &mut self,
        bytes: usize,
        resource: GeometryAllocation,
    ) -> Result<(), GeometryStorageError> {
        self.try_add_nested(0, bytes, resource)
    }

    fn try_merge(
        &mut self,
        nested: Self,
        resource: GeometryAllocation,
    ) -> Result<(), GeometryStorageError> {
        self.try_add_nested(nested.collection_slots, nested.bytes, resource)
    }
}

fn try_compact_geometry_vec<T>(
    values: Vec<T>,
    resource: GeometryAllocation,
) -> Result<Vec<T>, GeometryStorageError> {
    values
        .capacity()
        .checked_mul(size_of::<T>())
        .ok_or(GeometryStorageError::CollectionSizeOverflow { resource })?;
    let plan = CapacityPlan::for_type::<T>(0, 0, values.len())
        .map_err(|_| GeometryStorageError::CollectionSizeOverflow { resource })?;
    if values.capacity() <= plan.envelope_capacity() {
        return Ok(values);
    }

    if values.is_empty() {
        return Ok(Vec::new());
    }

    let mut compact = Vec::new();
    compact
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| GeometryStorageError::AllocationFailed { resource })?;
    plan.finish(compact.len(), compact.capacity())
        .map_err(|_| GeometryStorageError::AllocationFailed { resource })?;
    compact.extend(values);
    Ok(compact)
}

/// A collection cannot be addressed by the canonical dense-index domain.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error("{collection:?} count {count} exceeds the u32 geometry index domain")]
pub struct GeometryCountError {
    pub collection: GeometryCollection,
    pub count: usize,
}

fn checked_geometry_count(
    count: usize,
    collection: GeometryCollection,
) -> Result<u32, GeometryCountError> {
    u32::try_from(count).map_err(|_| GeometryCountError { collection, count })
}

/// Explicit corner counts for the faces of a polygon primitive.
///
/// An empty value represents an empty polygon primitive. Every present face has
/// at least three corners, and the cached total is checked while constructing
/// the value.
#[derive(PartialEq, Eq)]
pub struct PolygonBoundaries {
    face_corner_counts: Vec<u32>,
    face_count: u32,
    corner_count: u32,
}

impl fmt::Debug for PolygonBoundaries {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PolygonBoundaries")
            .field("face_count", &self.face_count)
            .field("corner_count", &self.corner_count)
            .finish()
    }
}

impl PolygonBoundaries {
    pub fn try_new(face_corner_counts: Vec<u32>) -> Result<Self, PolygonBoundariesError> {
        let face_count =
            checked_geometry_count(face_corner_counts.len(), GeometryCollection::PolygonFaces)?;
        let mut corner_count = 0_u32;
        for (face, &count) in face_corner_counts.iter().enumerate() {
            if count < 3 {
                return Err(PolygonBoundariesError::FaceTooSmall { face, count });
            }
            corner_count = corner_count
                .checked_add(count)
                .ok_or(PolygonBoundariesError::CornerCountOverflow { face })?;
        }
        let face_corner_counts =
            try_compact_geometry_vec(face_corner_counts, GeometryAllocation::PolygonFaces)?;

        Ok(Self {
            face_corner_counts,
            face_count,
            corner_count,
        })
    }

    pub fn face_corner_counts(&self) -> &[u32] {
        &self.face_corner_counts
    }

    pub const fn face_count(&self) -> u32 {
        self.face_count
    }

    pub const fn corner_count(&self) -> u32 {
        self.corner_count
    }

    pub fn retained_footprint(&self) -> Result<GeometryRetainedFootprint, GeometryStorageError> {
        let mut footprint = GeometryRetainedFootprint::default();
        footprint.try_add_capacity::<u32>(
            self.face_corner_counts.capacity(),
            GeometryAllocation::PolygonFaces,
        )?;
        Ok(footprint)
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PolygonBoundariesError {
    #[error(transparent)]
    Count(#[from] GeometryCountError),

    #[error("polygon face {face} has {count} corners; at least three are required")]
    FaceTooSmall { face: usize, count: u32 },

    #[error("polygon corner count overflows u32 while adding face {face}")]
    CornerCountOverflow { face: usize },

    #[error(transparent)]
    Storage(#[from] GeometryStorageError),
}

/// The producer-backed raw topology of one homogeneous primitive.
#[derive(Debug, PartialEq, Eq)]
pub enum Topology {
    Points,
    LineList,
    LineStrip,
    LineLoop,
    TriangleList,
    TriangleStrip,
    TriangleFan,
    Polygons(PolygonBoundaries),
}

/// A payload-free topology category suitable for diagnostics and reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TopologyKind {
    Points,
    LineList,
    LineStrip,
    LineLoop,
    TriangleList,
    TriangleStrip,
    TriangleFan,
    Polygons,
}

impl Topology {
    pub const fn kind(&self) -> TopologyKind {
        match self {
            Self::Points => TopologyKind::Points,
            Self::LineList => TopologyKind::LineList,
            Self::LineStrip => TopologyKind::LineStrip,
            Self::LineLoop => TopologyKind::LineLoop,
            Self::TriangleList => TopologyKind::TriangleList,
            Self::TriangleStrip => TopologyKind::TriangleStrip,
            Self::TriangleFan => TopologyKind::TriangleFan,
            Self::Polygons(_) => TopologyKind::Polygons,
        }
    }

    pub const fn polygon_boundaries(&self) -> Option<&PolygonBoundaries> {
        match self {
            Self::Polygons(boundaries) => Some(boundaries),
            _ => None,
        }
    }

    /// Computes face cardinality from raw corner cardinality.
    pub fn face_count(&self, corner_count: u32) -> Result<u32, TopologyError> {
        match self {
            Self::Points => Ok(corner_count),
            Self::LineList => list_face_count(self.kind(), corner_count, 2),
            Self::LineStrip => Ok(corner_count.saturating_sub(1)),
            Self::LineLoop if corner_count < 2 => Ok(0),
            Self::LineLoop => Ok(corner_count),
            Self::TriangleList => list_face_count(self.kind(), corner_count, 3),
            Self::TriangleStrip | Self::TriangleFan => Ok(corner_count.saturating_sub(2)),
            Self::Polygons(boundaries) if boundaries.corner_count() == corner_count => {
                Ok(boundaries.face_count())
            }
            Self::Polygons(boundaries) => Err(TopologyError::PolygonCornerCountMismatch {
                expected: boundaries.corner_count(),
                actual: corner_count,
            }),
        }
    }

    fn retained_footprint(&self) -> Result<GeometryRetainedFootprint, GeometryStorageError> {
        match self {
            Self::Polygons(boundaries) => boundaries.retained_footprint(),
            Self::Points
            | Self::LineList
            | Self::LineStrip
            | Self::LineLoop
            | Self::TriangleList
            | Self::TriangleStrip
            | Self::TriangleFan => Ok(GeometryRetainedFootprint::default()),
        }
    }
}

fn list_face_count(
    topology: TopologyKind,
    corner_count: u32,
    corners_per_face: u32,
) -> Result<u32, TopologyError> {
    if corner_count.is_multiple_of(corners_per_face) {
        Ok(corner_count / corners_per_face)
    } else {
        Err(TopologyError::ListCornerCountNotDivisible {
            topology,
            corner_count,
            corners_per_face,
        })
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum TopologyError {
    #[error("{topology:?} corner count {corner_count} is not divisible by {corners_per_face}")]
    ListCornerCountNotDivisible {
        topology: TopologyKind,
        corner_count: u32,
        corners_per_face: u32,
    },

    #[error(
        "polygon boundaries contain {expected} corners, but the index stream contains {actual}"
    )]
    PolygonCornerCountMismatch { expected: u32, actual: u32 },
}

enum IndexStorage {
    NonIndexed,
    Indexed { values: Vec<u32>, count: u32 },
}

/// An explicit indexed or non-indexed primitive vertex-address stream.
///
/// `Indexed([])` remains distinct from `NonIndexed`; an empty vector never
/// silently changes the indexing mode.
pub struct IndexStream {
    storage: IndexStorage,
}

impl IndexStream {
    pub const fn non_indexed() -> Self {
        Self {
            storage: IndexStorage::NonIndexed,
        }
    }

    pub fn try_indexed(values: Vec<u32>) -> Result<Self, GeometryStorageError> {
        let count = checked_geometry_count(values.len(), GeometryCollection::IndexElements)?;
        let values = try_compact_geometry_vec(values, GeometryAllocation::IndexElements)?;
        Ok(Self {
            storage: IndexStorage::Indexed { values, count },
        })
    }

    pub const fn is_indexed(&self) -> bool {
        matches!(&self.storage, IndexStorage::Indexed { .. })
    }

    pub fn indices(&self) -> Option<&[u32]> {
        match &self.storage {
            IndexStorage::NonIndexed => None,
            IndexStorage::Indexed { values, .. } => Some(values),
        }
    }

    pub const fn corner_count(&self, vertex_count: u32) -> u32 {
        match &self.storage {
            IndexStorage::NonIndexed => vertex_count,
            IndexStorage::Indexed { count, .. } => *count,
        }
    }

    fn validate_vertex_range(&self, vertex_count: u32) -> Result<(), IndexStreamError> {
        let Some(indices) = self.indices() else {
            return Ok(());
        };
        for (corner, &index) in indices.iter().enumerate() {
            if index >= vertex_count {
                return Err(IndexStreamError::IndexOutOfRange {
                    corner,
                    index,
                    vertex_count,
                });
            }
        }
        Ok(())
    }

    pub fn retained_footprint(&self) -> Result<GeometryRetainedFootprint, GeometryStorageError> {
        let mut footprint = GeometryRetainedFootprint::default();
        if let IndexStorage::Indexed { values, .. } = &self.storage {
            footprint
                .try_add_capacity::<u32>(values.capacity(), GeometryAllocation::IndexElements)?;
        }
        Ok(footprint)
    }
}

impl fmt::Debug for IndexStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.storage {
            IndexStorage::NonIndexed => formatter.write_str("IndexStream::NonIndexed"),
            IndexStorage::Indexed { count, .. } => formatter
                .debug_struct("IndexStream::Indexed")
                .field("count", count)
                .finish(),
        }
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum IndexStreamError {
    #[error(
        "index stream corner {corner} addresses vertex {index}, outside vertex count {vertex_count}"
    )]
    IndexOutOfRange {
        corner: usize,
        index: u32,
        vertex_count: u32,
    },
}

/// Cardinalities derived solely from a primitive's vertex count, index mode,
/// and topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrimitiveCardinality {
    vertex_count: u32,
    corner_count: u32,
    face_count: u32,
}

impl PrimitiveCardinality {
    fn try_new(
        vertex_count: u32,
        index_stream: &IndexStream,
        topology: &Topology,
    ) -> Result<Self, PrimitiveCardinalityError> {
        index_stream.validate_vertex_range(vertex_count)?;
        let corner_count = index_stream.corner_count(vertex_count);
        let face_count = topology.face_count(corner_count)?;
        Ok(Self {
            vertex_count,
            corner_count,
            face_count,
        })
    }

    pub const fn vertex_count(self) -> u32 {
        self.vertex_count
    }

    pub const fn corner_count(self) -> u32 {
        self.corner_count
    }

    pub const fn face_count(self) -> u32 {
        self.face_count
    }

    pub const fn domain_count(self, domain: AttributeDomain) -> Option<u32> {
        match domain {
            AttributeDomain::Vertex => Some(self.vertex_count),
            AttributeDomain::Corner => Some(self.corner_count),
            AttributeDomain::Face => Some(self.face_count),
            AttributeDomain::Instance => None,
        }
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveCardinalityError {
    #[error(transparent)]
    IndexStream(#[from] IndexStreamError),

    #[error(transparent)]
    Topology(#[from] TopologyError),
}

/// Which morph pose a derived bounds value encloses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MorphBoundsPolicy {
    /// Only the undeformed base geometry is enclosed.
    BaseGeometry,
    /// The logical mesh's default morph weights are applied.
    DefaultWeights,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoundsAxis {
    X,
    Y,
    Z,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoundsEndpoint {
    Minimum,
    Maximum,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum GeometryBoundsError {
    #[error("validated bounds construction requires a valid derived value")]
    MissingValidatedValue,

    #[error("bounds {endpoint:?} {axis:?} coordinate is not finite")]
    NonFinite {
        endpoint: BoundsEndpoint,
        axis: BoundsAxis,
    },

    #[error("bounds minimum exceeds its maximum on the {axis:?} axis")]
    Inverted { axis: BoundsAxis },
}

pub(crate) fn validate_aabb(bounds: Aabb) -> Result<(), GeometryBoundsError> {
    for (axis, minimum, maximum) in [
        (BoundsAxis::X, bounds.min.x, bounds.max.x),
        (BoundsAxis::Y, bounds.min.y, bounds.max.y),
        (BoundsAxis::Z, bounds.min.z, bounds.max.z),
    ] {
        if !minimum.is_finite() {
            return Err(GeometryBoundsError::NonFinite {
                endpoint: BoundsEndpoint::Minimum,
                axis,
            });
        }
        if !maximum.is_finite() {
            return Err(GeometryBoundsError::NonFinite {
                endpoint: BoundsEndpoint::Maximum,
                axis,
            });
        }
        if minimum > maximum {
            return Err(GeometryBoundsError::Inverted { axis });
        }
    }
    Ok(())
}

/// Primitive-local derived bounds. Its distinct owner type prevents accidental
/// substitution for logical-mesh or scene-space bounds.
#[derive(Debug, PartialEq)]
pub struct PrimitiveBounds {
    derived: Derived<Aabb>,
    morph_policy: MorphBoundsPolicy,
}

impl PrimitiveBounds {
    /// Wraps bounds whose generation and dependency stamp were minted by the
    /// central validator or an edit commit.
    pub(crate) fn try_from_validated(
        derived: Derived<Aabb>,
        morph_policy: MorphBoundsPolicy,
    ) -> Result<Self, GeometryBoundsError> {
        let bounds = derived
            .value()
            .copied()
            .ok_or(GeometryBoundsError::MissingValidatedValue)?;
        validate_aabb(bounds)?;
        Ok(Self {
            derived,
            morph_policy,
        })
    }

    pub const fn absent(reason: DerivedAbsenceReason, morph_policy: MorphBoundsPolicy) -> Self {
        Self {
            derived: Derived::absent(reason),
            morph_policy,
        }
    }

    pub const fn derived(&self) -> &Derived<Aabb> {
        &self.derived
    }

    pub const fn morph_policy(&self) -> MorphBoundsPolicy {
        self.morph_policy
    }
}

impl Default for PrimitiveBounds {
    fn default() -> Self {
        Self::absent(
            DerivedAbsenceReason::NotProvided,
            MorphBoundsPolicy::BaseGeometry,
        )
    }
}

/// Logical-mesh-local derived bounds. This cannot be passed where a primitive
/// bounds owner is required.
#[derive(Debug, PartialEq)]
pub struct LogicalMeshBounds {
    derived: Derived<Aabb>,
    morph_policy: MorphBoundsPolicy,
}

impl LogicalMeshBounds {
    /// Wraps bounds whose generation and dependency stamp were minted by the
    /// central validator or an edit commit.
    pub(crate) fn try_from_validated(
        derived: Derived<Aabb>,
        morph_policy: MorphBoundsPolicy,
    ) -> Result<Self, GeometryBoundsError> {
        let bounds = derived
            .value()
            .copied()
            .ok_or(GeometryBoundsError::MissingValidatedValue)?;
        validate_aabb(bounds)?;
        Ok(Self {
            derived,
            morph_policy,
        })
    }

    pub const fn absent(reason: DerivedAbsenceReason, morph_policy: MorphBoundsPolicy) -> Self {
        Self {
            derived: Derived::absent(reason),
            morph_policy,
        }
    }

    pub const fn derived(&self) -> &Derived<Aabb> {
        &self.derived
    }

    pub const fn morph_policy(&self) -> MorphBoundsPolicy {
        self.morph_policy
    }
}

impl Default for LogicalMeshBounds {
    fn default() -> Self {
        Self::absent(
            DerivedAbsenceReason::NotProvided,
            MorphBoundsPolicy::BaseGeometry,
        )
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveAttributeError {
    #[error(transparent)]
    Keys(#[from] AttributeKeyValidationError),

    #[error("primitive attribute {attribute} uses the Instance domain")]
    InstanceDomain { attribute: usize },

    #[error(
        "primitive attribute {attribute} in {domain:?} has cardinality {actual}, expected {expected}"
    )]
    Cardinality {
        attribute: usize,
        domain: AttributeDomain,
        expected: u32,
        actual: u32,
    },

    #[error("a primitive with {vertex_count} vertices requires a Vertex-domain POSITION attribute")]
    MissingPosition { vertex_count: u32 },
}

fn validate_primitive_attributes(
    attributes: &[Attribute],
    cardinality: PrimitiveCardinality,
) -> Result<(), PrimitiveAttributeError> {
    validate_unique_attribute_keys(attributes)?;
    let mut has_position = false;
    for (attribute_index, attribute) in attributes.iter().enumerate() {
        let domain = attribute.domain();
        let Some(expected) = cardinality.domain_count(domain) else {
            return Err(PrimitiveAttributeError::InstanceDomain {
                attribute: attribute_index,
            });
        };
        if let Err(error) = attribute.validate_cardinality(expected) {
            return Err(PrimitiveAttributeError::Cardinality {
                attribute: attribute_index,
                domain,
                expected: error.expected,
                actual: error.actual,
            });
        }
        has_position |= attribute.semantic() == &AttributeSemantic::Position;
    }

    if cardinality.vertex_count() > 0 && !has_position {
        return Err(PrimitiveAttributeError::MissingPosition {
            vertex_count: cardinality.vertex_count(),
        });
    }
    Ok(())
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveMorphSetError {
    #[error(transparent)]
    Count(#[from] GeometryCountError),

    #[error(
        "primitive morph stream {stream} targets local morph {target}, outside target count {target_count}"
    )]
    TargetOutOfRange {
        stream: usize,
        target: u32,
        target_count: u32,
    },

    #[error(
        "primitive morph stream {duplicate_stream} repeats target {target} first used by stream {first_stream}"
    )]
    DuplicateTarget {
        first_stream: usize,
        duplicate_stream: usize,
        target: u32,
    },

    #[error(
        "primitive morph stream {stream} has vertex count {actual}, expected primitive count {expected}"
    )]
    VertexCountMismatch {
        stream: usize,
        expected: u32,
        actual: u32,
    },

    #[error(
        "primitive morph stream {stream} explicitly targets local morph {target} without any delta"
    )]
    EmptyTarget { stream: usize, target: u32 },

    #[error(
        "primitive morph stream {stream} targets local morph {target} after stream {previous_stream} targeted {previous_target}; target order must be strictly increasing"
    )]
    NonCanonicalOrder {
        previous_stream: usize,
        previous_target: u32,
        stream: usize,
        target: u32,
    },
}

fn validate_primitive_morph_targets(
    morph_targets: &[PrimitiveMorphTarget],
    target_count: u32,
    vertex_count: u32,
) -> Result<(), PrimitiveMorphSetError> {
    checked_geometry_count(
        morph_targets.len(),
        GeometryCollection::PrimitiveMorphTargets,
    )?;
    let mut previous = None;

    for (stream, target) in morph_targets.iter().enumerate() {
        let local = target.target();
        let local_index = local.as_u32();
        if [
            MorphDeltaSemantic::Position,
            MorphDeltaSemantic::Normal,
            MorphDeltaSemantic::Tangent,
        ]
        .into_iter()
        .all(|semantic| target.delta(semantic).is_none())
        {
            return Err(PrimitiveMorphSetError::EmptyTarget {
                stream,
                target: local_index,
            });
        }
        if local.as_u32() >= target_count {
            return Err(PrimitiveMorphSetError::TargetOutOfRange {
                stream,
                target: local_index,
                target_count,
            });
        }
        if target.vertex_count() != vertex_count {
            return Err(PrimitiveMorphSetError::VertexCountMismatch {
                stream,
                expected: vertex_count,
                actual: target.vertex_count(),
            });
        }
        if let Some((previous_stream, previous_target)) = previous {
            if local_index == previous_target {
                return Err(PrimitiveMorphSetError::DuplicateTarget {
                    first_stream: previous_stream,
                    duplicate_stream: stream,
                    target: local_index,
                });
            }
            if local_index < previous_target {
                return Err(PrimitiveMorphSetError::NonCanonicalOrder {
                    previous_stream,
                    previous_target,
                    stream,
                    target: local_index,
                });
            }
        }
        previous = Some((stream, local_index));
    }
    Ok(())
}

/// One homogeneous primitive owned by exactly one logical mesh.
///
/// Cross-entity references use crate-private storage indexes. A public Scene
/// read view qualifies them with the owning generation and logical mesh.
pub struct MeshPrimitive {
    topology: Topology,
    index_stream: IndexStream,
    cardinality: PrimitiveCardinality,
    material: Option<MaterialIndex>,
    attributes: Vec<Attribute>,
    influences: Option<InfluenceRows>,
    morph_targets: Vec<PrimitiveMorphTarget>,
    bounds: PrimitiveBounds,
    provenance: ProvenanceAttachments,
}

impl MeshPrimitive {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_new(
        vertex_count: u32,
        topology: Topology,
        index_stream: IndexStream,
        material: Option<MaterialIndex>,
        attributes: Vec<Attribute>,
        influences: Option<InfluenceRows>,
        morph_targets: Vec<PrimitiveMorphTarget>,
        logical_morph_target_count: u32,
        bounds: PrimitiveBounds,
        provenance: impl Into<ProvenanceAttachments>,
    ) -> Result<Self, MeshPrimitiveError> {
        let cardinality = PrimitiveCardinality::try_new(vertex_count, &index_stream, &topology)?;
        validate_primitive_attributes(&attributes, cardinality)?;
        if let Some(influences) = &influences
            && influences.vertex_count() != vertex_count
        {
            return Err(MeshPrimitiveError::InfluenceVertexCountMismatch {
                expected: vertex_count,
                actual: influences.vertex_count(),
            });
        }
        validate_primitive_morph_targets(&morph_targets, logical_morph_target_count, vertex_count)?;
        if vertex_count == 0 && bounds.derived().is_valid() {
            return Err(MeshPrimitiveError::BoundsOnEmptyVertexDomain);
        }
        let attributes =
            try_compact_geometry_vec(attributes, GeometryAllocation::PrimitiveAttributes)?;
        let morph_targets =
            try_compact_geometry_vec(morph_targets, GeometryAllocation::PrimitiveMorphTargets)?;
        let provenance = provenance.into();

        Ok(Self {
            topology,
            index_stream,
            cardinality,
            material,
            attributes,
            influences,
            morph_targets,
            bounds,
            provenance,
        })
    }

    pub const fn topology(&self) -> &Topology {
        &self.topology
    }

    pub const fn index_stream(&self) -> &IndexStream {
        &self.index_stream
    }

    pub const fn cardinality(&self) -> PrimitiveCardinality {
        self.cardinality
    }

    pub const fn domain_count(&self, domain: AttributeDomain) -> Option<u32> {
        self.cardinality.domain_count(domain)
    }

    pub const fn has_material(&self) -> bool {
        self.material.is_some()
    }

    pub(crate) const fn material_index(&self) -> Option<MaterialIndex> {
        self.material
    }

    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    pub const fn influences(&self) -> Option<&InfluenceRows> {
        self.influences.as_ref()
    }

    pub fn morph_targets(&self) -> &[PrimitiveMorphTarget] {
        &self.morph_targets
    }

    pub const fn bounds(&self) -> &PrimitiveBounds {
        &self.bounds
    }

    pub const fn provenance(&self) -> &ProvenanceAttachments {
        &self.provenance
    }

    pub fn retained_footprint(&self) -> Result<GeometryRetainedFootprint, GeometryStorageError> {
        unwrap_uncontrolled_footprint(self.retained_footprint_controlled(&mut || Ok(())))
    }

    pub(crate) fn retained_footprint_controlled<E>(
        &self,
        charge_work: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<GeometryRetainedFootprint, ControlledGeometryStorageError<E>> {
        let mut footprint = self.topology.retained_footprint()?;
        footprint.try_merge(
            self.index_stream.retained_footprint()?,
            GeometryAllocation::IndexElements,
        )?;
        footprint.try_add_capacity::<Attribute>(
            self.attributes.capacity(),
            GeometryAllocation::PrimitiveAttributes,
        )?;
        for attribute in &self.attributes {
            charge_work().map_err(ControlledGeometryStorageError::Control)?;
            footprint.try_add_bytes(
                attribute.payload().retained_allocation_bytes(),
                GeometryAllocation::PrimitiveAttributes,
            )?;
        }
        if let Some(influences) = &self.influences {
            footprint.try_add_nested(
                influences.retained_cell_capacity(),
                influences.retained_allocation_bytes(),
                GeometryAllocation::PrimitiveInfluences,
            )?;
        }
        footprint.try_add_capacity::<PrimitiveMorphTarget>(
            self.morph_targets.capacity(),
            GeometryAllocation::PrimitiveMorphTargets,
        )?;
        for target in &self.morph_targets {
            charge_work().map_err(ControlledGeometryStorageError::Control)?;
            footprint.try_add_nested(
                target.retained_cell_capacity(),
                target.retained_allocation_bytes(),
                GeometryAllocation::PrimitiveMorphTargets,
            )?;
        }
        footprint.try_add_bytes(
            self.bounds
                .derived()
                .retained_producer_allocation_bytes()
                .map_err(|_| GeometryStorageError::RetainedFootprintOverflow {
                    resource: GeometryAllocation::PrimitiveBoundsProducer,
                })?,
            GeometryAllocation::PrimitiveBoundsProducer,
        )?;
        footprint.try_add_nested(
            self.provenance.retained_allocation_cells(),
            self.provenance.retained_allocation_bytes(),
            GeometryAllocation::PrimitiveProvenanceAttachments,
        )?;
        Ok(footprint)
    }
}

impl fmt::Debug for MeshPrimitive {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MeshPrimitive")
            .field("topology", &self.topology.kind())
            .field("indexed", &self.index_stream.is_indexed())
            .field("cardinality", &self.cardinality)
            .field("has_material", &self.material.is_some())
            .field("attribute_count", &self.attributes.len())
            .field("has_influences", &self.influences.is_some())
            .field("morph_target_count", &self.morph_targets.len())
            .field("bounds", &self.bounds)
            .field("provenance", &self.provenance)
            .finish()
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MeshPrimitiveError {
    #[error(transparent)]
    Cardinality(#[from] PrimitiveCardinalityError),

    #[error(transparent)]
    Attributes(#[from] PrimitiveAttributeError),

    #[error("influence rows have vertex count {actual}, expected {expected}")]
    InfluenceVertexCountMismatch { expected: u32, actual: u32 },

    #[error(transparent)]
    MorphTargets(#[from] PrimitiveMorphSetError),

    #[error("an empty primitive cannot carry valid derived bounds")]
    BoundsOnEmptyVertexDomain,

    #[error(transparent)]
    Storage(#[from] GeometryStorageError),
}

/// One source-level mesh identity owning one or more homogeneous primitives.
pub struct LogicalMesh {
    name: Option<EntityName>,
    primitives: Vec<MeshPrimitive>,
    morph_targets: Vec<MorphTargetDefinition>,
    default_morph_weights: Vec<f32>,
    bounds: LogicalMeshBounds,
    provenance: ProvenanceAttachments,
}

impl LogicalMesh {
    pub(crate) fn try_new(
        name: Option<EntityName>,
        primitives: Vec<MeshPrimitive>,
        morph_targets: Vec<MorphTargetDefinition>,
        default_morph_weights: Vec<f32>,
        bounds: LogicalMeshBounds,
        provenance: impl Into<ProvenanceAttachments>,
    ) -> Result<Self, LogicalMeshError> {
        if primitives.is_empty() {
            return Err(LogicalMeshError::NoPrimitives);
        }
        checked_geometry_count(primitives.len(), GeometryCollection::LogicalMeshPrimitives)?;
        let morph_target_count = checked_geometry_count(
            morph_targets.len(),
            GeometryCollection::LogicalMeshMorphTargets,
        )?;
        if default_morph_weights.len() != morph_targets.len() {
            return Err(LogicalMeshError::DefaultMorphWeightCountMismatch {
                expected: morph_targets.len(),
                actual: default_morph_weights.len(),
            });
        }
        if let Some(index) = default_morph_weights
            .iter()
            .position(|weight| !weight.is_finite())
        {
            return Err(LogicalMeshError::NonFiniteDefaultMorphWeight { index });
        }
        for (primitive, value) in primitives.iter().enumerate() {
            validate_primitive_morph_targets(
                value.morph_targets(),
                morph_target_count,
                value.cardinality().vertex_count(),
            )
            .map_err(|source| LogicalMeshError::PrimitiveMorphTargets { primitive, source })?;
        }
        let has_vertices = primitives
            .iter()
            .any(|primitive| primitive.cardinality().vertex_count() > 0);
        if !has_vertices && bounds.derived().is_valid() {
            return Err(LogicalMeshError::BoundsOnEmptyGeometry);
        }
        let primitives =
            try_compact_geometry_vec(primitives, GeometryAllocation::LogicalMeshPrimitives)?;
        let morph_targets =
            try_compact_geometry_vec(morph_targets, GeometryAllocation::LogicalMeshMorphTargets)?;
        let default_morph_weights = try_compact_geometry_vec(
            default_morph_weights,
            GeometryAllocation::LogicalMeshDefaultMorphWeights,
        )?;
        let provenance = provenance.into();

        Ok(Self {
            name,
            primitives,
            morph_targets,
            default_morph_weights,
            bounds,
            provenance,
        })
    }

    pub const fn name(&self) -> Option<&EntityName> {
        self.name.as_ref()
    }

    pub fn primitives(&self) -> &[MeshPrimitive] {
        &self.primitives
    }

    pub fn morph_target_definitions(&self) -> &[MorphTargetDefinition] {
        &self.morph_targets
    }

    pub fn default_morph_weights(&self) -> &[f32] {
        &self.default_morph_weights
    }

    pub const fn bounds(&self) -> &LogicalMeshBounds {
        &self.bounds
    }

    pub const fn provenance(&self) -> &ProvenanceAttachments {
        &self.provenance
    }

    pub fn retained_footprint(&self) -> Result<GeometryRetainedFootprint, GeometryStorageError> {
        unwrap_uncontrolled_footprint(self.retained_footprint_controlled(&mut || Ok(())))
    }

    pub(crate) fn retained_footprint_controlled<E>(
        &self,
        charge_work: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<GeometryRetainedFootprint, ControlledGeometryStorageError<E>> {
        let mut footprint = GeometryRetainedFootprint::default();
        if let Some(name) = &self.name {
            footprint.try_add_bytes(name.capacity(), GeometryAllocation::LogicalMeshName)?;
        }
        footprint.try_add_capacity::<MeshPrimitive>(
            self.primitives.capacity(),
            GeometryAllocation::LogicalMeshPrimitives,
        )?;
        for primitive in &self.primitives {
            charge_work().map_err(ControlledGeometryStorageError::Control)?;
            footprint.try_merge(
                primitive.retained_footprint_controlled(charge_work)?,
                GeometryAllocation::LogicalMeshPrimitives,
            )?;
        }
        footprint.try_add_capacity::<MorphTargetDefinition>(
            self.morph_targets.capacity(),
            GeometryAllocation::LogicalMeshMorphTargets,
        )?;
        for target in &self.morph_targets {
            charge_work().map_err(ControlledGeometryStorageError::Control)?;
            footprint.try_add_bytes(
                target.retained_allocation_bytes(),
                GeometryAllocation::LogicalMeshMorphTargets,
            )?;
        }
        footprint.try_add_capacity::<f32>(
            self.default_morph_weights.capacity(),
            GeometryAllocation::LogicalMeshDefaultMorphWeights,
        )?;
        footprint.try_add_bytes(
            self.bounds
                .derived()
                .retained_producer_allocation_bytes()
                .map_err(|_| GeometryStorageError::RetainedFootprintOverflow {
                    resource: GeometryAllocation::LogicalMeshBoundsProducer,
                })?,
            GeometryAllocation::LogicalMeshBoundsProducer,
        )?;
        footprint.try_add_nested(
            self.provenance.retained_allocation_cells(),
            self.provenance.retained_allocation_bytes(),
            GeometryAllocation::LogicalMeshProvenanceAttachments,
        )?;
        Ok(footprint)
    }
}

impl fmt::Debug for LogicalMesh {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LogicalMesh")
            .field("name", &self.name)
            .field("primitive_count", &self.primitives.len())
            .field("morph_target_count", &self.morph_targets.len())
            .field(
                "default_morph_weight_count",
                &self.default_morph_weights.len(),
            )
            .field("bounds", &self.bounds)
            .field("provenance", &self.provenance)
            .finish()
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum LogicalMeshError {
    #[error("a logical mesh must own at least one primitive")]
    NoPrimitives,

    #[error(transparent)]
    Count(#[from] GeometryCountError),

    #[error("default morph weight count is {actual}, expected {expected}")]
    DefaultMorphWeightCountMismatch { expected: usize, actual: usize },

    #[error("default morph weight {index} is not finite")]
    NonFiniteDefaultMorphWeight { index: usize },

    #[error("primitive {primitive} has invalid logical morph-target ownership")]
    PrimitiveMorphTargets {
        primitive: usize,
        #[source]
        source: PrimitiveMorphSetError,
    },

    #[error("a logical mesh with no vertices cannot carry valid derived bounds")]
    BoundsOnEmptyGeometry,

    #[error(transparent)]
    Storage(#[from] GeometryStorageError),
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MeshInstanceAttributeError {
    #[error(transparent)]
    Keys(#[from] AttributeKeyValidationError),

    #[error("mesh-instance attribute {attribute} uses {domain:?}; Instance is required")]
    InvalidDomain {
        attribute: usize,
        domain: AttributeDomain,
    },

    #[error("mesh-instance attribute {attribute} has cardinality {actual}, expected one")]
    Cardinality { attribute: usize, actual: u32 },
}

fn validate_instance_attributes(
    attributes: &[Attribute],
) -> Result<(), MeshInstanceAttributeError> {
    validate_unique_attribute_keys(attributes)?;
    for (attribute_index, attribute) in attributes.iter().enumerate() {
        if attribute.domain() != AttributeDomain::Instance {
            return Err(MeshInstanceAttributeError::InvalidDomain {
                attribute: attribute_index,
                domain: attribute.domain(),
            });
        }
        if let Err(error) = attribute.validate_cardinality(1) {
            return Err(MeshInstanceAttributeError::Cardinality {
                attribute: attribute_index,
                actual: error.actual,
            });
        }
    }
    Ok(())
}

/// One scene-owned placement of a logical mesh.
///
/// The canonical owner, mesh, and optional skin edges are private storage
/// indexes without generation state. Public handles are derived only by the
/// owning Scene read view.
pub struct MeshInstance {
    owner: NodeIndex,
    logical_mesh: LogicalMeshIndex,
    skin: Option<SkinIndex>,
    morph_weight_override: Option<Vec<f32>>,
    attributes: Vec<Attribute>,
    provenance: ProvenanceAttachments,
}

impl MeshInstance {
    pub(crate) fn try_new(
        owner: NodeIndex,
        logical_mesh: LogicalMeshIndex,
        skin: Option<SkinIndex>,
        logical_morph_target_count: u32,
        morph_weight_override: Option<Vec<f32>>,
        attributes: Vec<Attribute>,
        provenance: impl Into<ProvenanceAttachments>,
    ) -> Result<Self, MeshInstanceError> {
        validate_instance_attributes(&attributes)?;
        if let Some(weights) = &morph_weight_override {
            let target_count = usize::try_from(logical_morph_target_count).map_err(|_| {
                MeshInstanceError::MorphTargetCountUnsupported {
                    count: logical_morph_target_count,
                }
            })?;
            validate_morph_weight_override(target_count, weights)?;
        }
        let morph_weight_override = morph_weight_override
            .map(|weights| {
                try_compact_geometry_vec(weights, GeometryAllocation::MeshInstanceMorphWeights)
            })
            .transpose()?;
        let attributes =
            try_compact_geometry_vec(attributes, GeometryAllocation::MeshInstanceAttributes)?;
        let provenance = provenance.into();

        Ok(Self {
            owner,
            logical_mesh,
            skin,
            morph_weight_override,
            attributes,
            provenance,
        })
    }

    pub(crate) const fn owner_index(&self) -> NodeIndex {
        self.owner
    }

    pub(crate) const fn logical_mesh_index(&self) -> LogicalMeshIndex {
        self.logical_mesh
    }

    pub const fn has_skin(&self) -> bool {
        self.skin.is_some()
    }

    pub(crate) const fn skin_index(&self) -> Option<SkinIndex> {
        self.skin
    }

    pub fn morph_weight_override(&self) -> Option<&[f32]> {
        self.morph_weight_override.as_deref()
    }

    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    pub const fn domain_count(&self, domain: AttributeDomain) -> Option<u32> {
        match domain {
            AttributeDomain::Instance => Some(1),
            AttributeDomain::Vertex | AttributeDomain::Corner | AttributeDomain::Face => None,
        }
    }

    pub const fn provenance(&self) -> &ProvenanceAttachments {
        &self.provenance
    }

    pub fn retained_footprint(&self) -> Result<GeometryRetainedFootprint, GeometryStorageError> {
        unwrap_uncontrolled_footprint(self.retained_footprint_controlled(&mut || Ok(())))
    }

    pub(crate) fn retained_footprint_controlled<E>(
        &self,
        charge_work: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<GeometryRetainedFootprint, ControlledGeometryStorageError<E>> {
        let mut footprint = GeometryRetainedFootprint::default();
        if let Some(weights) = &self.morph_weight_override {
            footprint.try_add_capacity::<f32>(
                weights.capacity(),
                GeometryAllocation::MeshInstanceMorphWeights,
            )?;
        }
        footprint.try_add_capacity::<Attribute>(
            self.attributes.capacity(),
            GeometryAllocation::MeshInstanceAttributes,
        )?;
        for attribute in &self.attributes {
            charge_work().map_err(ControlledGeometryStorageError::Control)?;
            footprint.try_add_bytes(
                attribute.payload().retained_allocation_bytes(),
                GeometryAllocation::MeshInstanceAttributes,
            )?;
        }
        footprint.try_add_nested(
            self.provenance.retained_allocation_cells(),
            self.provenance.retained_allocation_bytes(),
            GeometryAllocation::MeshInstanceProvenanceAttachments,
        )?;
        Ok(footprint)
    }
}

impl fmt::Debug for MeshInstance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MeshInstance")
            .field("has_skin", &self.skin.is_some())
            .field(
                "morph_weight_override_count",
                &self.morph_weight_override.as_ref().map(Vec::len),
            )
            .field("attribute_count", &self.attributes.len())
            .field("provenance", &self.provenance)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MeshInstanceError {
    #[error(transparent)]
    Attributes(#[from] MeshInstanceAttributeError),

    #[error("logical morph-target count {count} cannot be represented on this platform")]
    MorphTargetCountUnsupported { count: u32 },

    #[error(transparent)]
    MorphWeightOverride(#[from] MorphWeightOverrideError),

    #[error(transparent)]
    Storage(#[from] GeometryStorageError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attribute::{
        AttributeBuffer, AttributeFormat, AttributePayload, ComponentType, Normalization,
    };
    use crate::deform::{Influence, MorphDelta, MorphDeltaSemantic};
    use crate::derived::{DerivedProducer, InputStamp};
    use crate::identity::{MaterialIndex, MorphTargetLocalIndex, SceneGenerationId, SkinIndex};
    use crate::math::{Vec3, Vec4};
    use crate::provenance::{
        Provenance, ProvenanceAttachments, test_provenance_fixture, test_provenance_pair_fixture,
    };

    fn provenance() -> Provenance {
        let (_arena, provenance) = test_provenance_fixture();
        provenance
    }

    fn format(component_type: ComponentType, lanes: u8) -> AttributeFormat {
        AttributeFormat::new(component_type, lanes, Normalization::None, None).unwrap()
    }

    fn positions(count: usize, provenance: Provenance) -> Attribute {
        Attribute::new(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            format(ComponentType::F32, 3),
            AttributePayload::dense(
                AttributeBuffer::from_f32_vec3(vec![Vec3::ZERO; count]).unwrap(),
            ),
            provenance,
        )
        .unwrap()
    }

    fn corner_normals(count: usize, provenance: Provenance) -> Attribute {
        Attribute::new(
            AttributeSemantic::Normal,
            AttributeDomain::Corner,
            format(ComponentType::F32, 3),
            AttributePayload::dense(
                AttributeBuffer::from_f32_vec3(vec![Vec3::new(0.0, 0.0, 1.0); count]).unwrap(),
            ),
            provenance,
        )
        .unwrap()
    }

    fn face_smoothing(count: usize, provenance: Provenance) -> Attribute {
        let smoothing_format = format(ComponentType::U32, 1);
        Attribute::new(
            AttributeSemantic::SmoothingGroup,
            AttributeDomain::Face,
            smoothing_format.clone(),
            AttributePayload::dense(
                AttributeBuffer::from_u32(smoothing_format, vec![0; count]).unwrap(),
            ),
            provenance,
        )
        .unwrap()
    }

    fn instance_color(count: usize, provenance: Provenance) -> Attribute {
        Attribute::new(
            AttributeSemantic::Color(0),
            AttributeDomain::Instance,
            format(ComponentType::F32, 4),
            AttributePayload::dense(
                AttributeBuffer::from_f32_vec4(vec![Vec4::new(1.0, 0.5, 0.25, 1.0); count])
                    .unwrap(),
            ),
            provenance,
        )
        .unwrap()
    }

    fn empty_morph_target(target: u32, vertex_count: u32) -> PrimitiveMorphTarget {
        PrimitiveMorphTarget::try_new(
            MorphTargetLocalIndex::from_u32(target),
            vertex_count,
            std::iter::empty::<(MorphDeltaSemantic, MorphDelta)>(),
        )
        .unwrap()
    }

    fn position_morph_target(
        target: u32,
        vertex_count: u32,
        provenance: Provenance,
    ) -> PrimitiveMorphTarget {
        let delta = MorphDelta::try_new(
            AttributePayload::dense(
                AttributeBuffer::from_f32_vec3(vec![Vec3::ZERO; vertex_count as usize]).unwrap(),
            ),
            provenance,
        )
        .unwrap();
        PrimitiveMorphTarget::try_new(
            MorphTargetLocalIndex::from_u32(target),
            vertex_count,
            [(MorphDeltaSemantic::Position, delta)],
        )
        .unwrap()
    }

    fn triangle_primitive(
        material: Option<MaterialIndex>,
        logical_morph_target_count: u32,
        morph_targets: Vec<PrimitiveMorphTarget>,
        provenance: Provenance,
    ) -> Result<MeshPrimitive, MeshPrimitiveError> {
        MeshPrimitive::try_new(
            3,
            Topology::TriangleList,
            IndexStream::non_indexed(),
            material,
            vec![positions(3, provenance)],
            None,
            morph_targets,
            logical_morph_target_count,
            PrimitiveBounds::default(),
            provenance,
        )
    }

    fn primitive_with_owner_provenance(
        attribute_provenance: Provenance,
        owner_provenance: impl Into<ProvenanceAttachments>,
    ) -> MeshPrimitive {
        MeshPrimitive::try_new(
            3,
            Topology::TriangleList,
            IndexStream::non_indexed(),
            None,
            vec![positions(3, attribute_provenance)],
            None,
            Vec::new(),
            0,
            PrimitiveBounds::default(),
            owner_provenance,
        )
        .unwrap()
    }

    #[test]
    fn derives_face_counts_for_every_raw_topology() {
        let cases = [
            (Topology::Points, 4, 4),
            (Topology::LineList, 4, 2),
            (Topology::LineStrip, 4, 3),
            (Topology::LineLoop, 0, 0),
            (Topology::LineLoop, 1, 0),
            (Topology::LineLoop, 2, 2),
            (Topology::LineLoop, 4, 4),
            (Topology::TriangleList, 6, 2),
            (Topology::TriangleStrip, 4, 2),
            (Topology::TriangleFan, 4, 2),
        ];
        for (topology, corners, expected_faces) in cases {
            assert_eq!(topology.face_count(corners), Ok(expected_faces));
        }

        let polygons = Topology::Polygons(PolygonBoundaries::try_new(vec![3, 4]).unwrap());
        assert_eq!(polygons.face_count(7), Ok(2));
        assert_eq!(
            Topology::LineList.face_count(3),
            Err(TopologyError::ListCornerCountNotDivisible {
                topology: TopologyKind::LineList,
                corner_count: 3,
                corners_per_face: 2,
            })
        );
        assert_eq!(
            Topology::TriangleList.face_count(4),
            Err(TopologyError::ListCornerCountNotDivisible {
                topology: TopologyKind::TriangleList,
                corner_count: 4,
                corners_per_face: 3,
            })
        );
    }

    #[test]
    fn strip_and_fan_boundaries_do_not_shift_face_or_domain_counts() {
        let boundary_cases = [
            (Topology::LineStrip, 0, 0),
            (Topology::LineStrip, 1, 0),
            (Topology::LineStrip, 2, 1),
            (Topology::TriangleStrip, 0, 0),
            (Topology::TriangleStrip, 1, 0),
            (Topology::TriangleStrip, 2, 0),
            (Topology::TriangleStrip, 3, 1),
            (Topology::TriangleFan, 0, 0),
            (Topology::TriangleFan, 1, 0),
            (Topology::TriangleFan, 2, 0),
            (Topology::TriangleFan, 3, 1),
        ];
        for (topology, corners, expected_faces) in boundary_cases {
            assert_eq!(topology.face_count(corners), Ok(expected_faces));
        }

        let provenance = provenance();
        let strip = MeshPrimitive::try_new(
            4,
            Topology::TriangleStrip,
            IndexStream::non_indexed(),
            None,
            vec![
                positions(4, provenance),
                corner_normals(4, provenance),
                face_smoothing(2, provenance),
            ],
            None,
            Vec::new(),
            0,
            PrimitiveBounds::default(),
            provenance,
        )
        .unwrap();
        assert_eq!(strip.domain_count(AttributeDomain::Corner), Some(4));
        assert_eq!(strip.domain_count(AttributeDomain::Face), Some(2));

        let off_by_one = MeshPrimitive::try_new(
            4,
            Topology::TriangleFan,
            IndexStream::non_indexed(),
            None,
            vec![positions(4, provenance), face_smoothing(3, provenance)],
            None,
            Vec::new(),
            0,
            PrimitiveBounds::default(),
            provenance,
        );
        assert!(matches!(
            off_by_one,
            Err(MeshPrimitiveError::Attributes(
                PrimitiveAttributeError::Cardinality {
                    attribute: 1,
                    domain: AttributeDomain::Face,
                    expected: 2,
                    actual: 3,
                }
            ))
        ));
    }

    #[test]
    fn polygon_boundaries_check_face_size_sum_and_stream_match() {
        assert_eq!(
            PolygonBoundaries::try_new(vec![2]),
            Err(PolygonBoundariesError::FaceTooSmall { face: 0, count: 2 })
        );
        assert_eq!(
            PolygonBoundaries::try_new(vec![u32::MAX, 3]),
            Err(PolygonBoundariesError::CornerCountOverflow { face: 1 })
        );
        let topology = Topology::Polygons(PolygonBoundaries::try_new(vec![3, 5]).unwrap());
        assert_eq!(
            topology.face_count(7),
            Err(TopologyError::PolygonCornerCountMismatch {
                expected: 8,
                actual: 7,
            })
        );
    }

    #[test]
    fn indexed_and_non_indexed_streams_have_explicit_distinct_cardinality() {
        let non_indexed = IndexStream::non_indexed();
        assert!(!non_indexed.is_indexed());
        assert_eq!(non_indexed.indices(), None);
        assert_eq!(non_indexed.corner_count(3), 3);

        let empty_indexed = IndexStream::try_indexed(Vec::new()).unwrap();
        assert!(empty_indexed.is_indexed());
        assert_eq!(empty_indexed.indices(), Some([].as_slice()));
        assert_eq!(empty_indexed.corner_count(3), 0);

        let indexed = IndexStream::try_indexed(vec![2, 0, 1, 2]).unwrap();
        assert_eq!(indexed.corner_count(3), 4);
        assert_eq!(indexed.validate_vertex_range(3), Ok(()));
        let debug = format!("{indexed:?}");
        assert!(debug.contains("count: 4"));
        assert!(!debug.contains("2, 0, 1, 2"));
        assert_eq!(
            indexed.validate_vertex_range(2),
            Err(IndexStreamError::IndexOutOfRange {
                corner: 0,
                index: 2,
                vertex_count: 2,
            })
        );
    }

    #[test]
    fn primitive_owns_domain_data_in_its_derived_cardinalities() {
        let provenance = provenance();
        let influences = InfluenceRows::try_new(
            4,
            vec![0, 0, 1, 1, 2],
            vec![Influence::new(0, 1.0, 0), Influence::new(1, 0.5, 0)],
        )
        .unwrap();
        let morph_delta = MorphDelta::try_new(
            AttributePayload::dense(AttributeBuffer::from_f32_vec3(vec![Vec3::ZERO; 4]).unwrap()),
            provenance,
        )
        .unwrap();
        let morph = PrimitiveMorphTarget::try_new(
            MorphTargetLocalIndex::from_u32(0),
            4,
            [(MorphDeltaSemantic::Position, morph_delta)],
        )
        .unwrap();
        let primitive = MeshPrimitive::try_new(
            4,
            Topology::Polygons(PolygonBoundaries::try_new(vec![4]).unwrap()),
            IndexStream::try_indexed(vec![0, 1, 2, 3]).unwrap(),
            Some(MaterialIndex::from_u32(2)),
            vec![
                positions(4, provenance),
                corner_normals(4, provenance),
                face_smoothing(1, provenance),
            ],
            Some(influences),
            vec![morph],
            1,
            PrimitiveBounds::default(),
            provenance,
        )
        .unwrap();

        assert_eq!(primitive.domain_count(AttributeDomain::Vertex), Some(4));
        assert_eq!(primitive.domain_count(AttributeDomain::Corner), Some(4));
        assert_eq!(primitive.domain_count(AttributeDomain::Face), Some(1));
        assert_eq!(primitive.domain_count(AttributeDomain::Instance), None);
        assert_eq!(primitive.material_index(), Some(MaterialIndex::from_u32(2)));
        assert_eq!(primitive.influences().unwrap().vertex_count(), 4);
        assert_eq!(primitive.morph_targets().len(), 1);
    }

    #[test]
    fn primitive_rejects_invalid_attribute_placement_cardinality_and_keys() {
        let provenance = provenance();
        let wrong_cardinality = MeshPrimitive::try_new(
            3,
            Topology::TriangleList,
            IndexStream::non_indexed(),
            None,
            vec![positions(2, provenance)],
            None,
            Vec::new(),
            0,
            PrimitiveBounds::default(),
            provenance,
        );
        assert!(matches!(
            wrong_cardinality,
            Err(MeshPrimitiveError::Attributes(
                PrimitiveAttributeError::Cardinality {
                    attribute: 0,
                    domain: AttributeDomain::Vertex,
                    expected: 3,
                    actual: 2,
                }
            ))
        ));

        let duplicate = MeshPrimitive::try_new(
            3,
            Topology::TriangleList,
            IndexStream::non_indexed(),
            None,
            vec![positions(3, provenance), positions(3, provenance)],
            None,
            Vec::new(),
            0,
            PrimitiveBounds::default(),
            provenance,
        );
        assert!(matches!(
            duplicate,
            Err(MeshPrimitiveError::Attributes(
                PrimitiveAttributeError::Keys(AttributeKeyValidationError::Duplicate(_))
            ))
        ));

        let instance_domain = MeshPrimitive::try_new(
            3,
            Topology::TriangleList,
            IndexStream::non_indexed(),
            None,
            vec![positions(3, provenance), instance_color(1, provenance)],
            None,
            Vec::new(),
            0,
            PrimitiveBounds::default(),
            provenance,
        );
        assert!(matches!(
            instance_domain,
            Err(MeshPrimitiveError::Attributes(
                PrimitiveAttributeError::InstanceDomain { attribute: 1 }
            ))
        ));

        let missing_position = MeshPrimitive::try_new(
            3,
            Topology::TriangleList,
            IndexStream::non_indexed(),
            None,
            Vec::new(),
            None,
            Vec::new(),
            0,
            PrimitiveBounds::default(),
            provenance,
        );
        assert!(matches!(
            missing_position,
            Err(MeshPrimitiveError::Attributes(
                PrimitiveAttributeError::MissingPosition { vertex_count: 3 }
            ))
        ));
    }

    #[test]
    fn primitive_checks_influence_and_morph_ownership() {
        let provenance = provenance();
        let influences = InfluenceRows::try_new(2, vec![0, 0, 0], Vec::new()).unwrap();
        let mismatch = MeshPrimitive::try_new(
            3,
            Topology::TriangleList,
            IndexStream::non_indexed(),
            None,
            vec![positions(3, provenance)],
            Some(influences),
            Vec::new(),
            0,
            PrimitiveBounds::default(),
            provenance,
        );
        assert!(matches!(
            mismatch,
            Err(MeshPrimitiveError::InfluenceVertexCountMismatch {
                expected: 3,
                actual: 2,
            })
        ));

        let explicitly_empty =
            triangle_primitive(None, 1, vec![empty_morph_target(0, 3)], provenance);
        assert!(matches!(
            explicitly_empty,
            Err(MeshPrimitiveError::MorphTargets(
                PrimitiveMorphSetError::EmptyTarget {
                    stream: 0,
                    target: 0,
                }
            ))
        ));

        let out_of_range = triangle_primitive(
            None,
            1,
            vec![position_morph_target(1, 3, provenance)],
            provenance,
        );
        assert!(matches!(
            out_of_range,
            Err(MeshPrimitiveError::MorphTargets(
                PrimitiveMorphSetError::TargetOutOfRange {
                    stream: 0,
                    target: 1,
                    target_count: 1,
                }
            ))
        ));

        let duplicate = triangle_primitive(
            None,
            1,
            vec![
                position_morph_target(0, 3, provenance),
                position_morph_target(0, 3, provenance),
            ],
            provenance,
        );
        assert!(matches!(
            duplicate,
            Err(MeshPrimitiveError::MorphTargets(
                PrimitiveMorphSetError::DuplicateTarget {
                    first_stream: 0,
                    duplicate_stream: 1,
                    target: 0,
                }
            ))
        ));

        let non_canonical = triangle_primitive(
            None,
            2,
            vec![
                position_morph_target(1, 3, provenance),
                position_morph_target(0, 3, provenance),
            ],
            provenance,
        );
        assert!(matches!(
            non_canonical,
            Err(MeshPrimitiveError::MorphTargets(
                PrimitiveMorphSetError::NonCanonicalOrder {
                    previous_stream: 0,
                    previous_target: 1,
                    stream: 1,
                    target: 0,
                }
            ))
        ));

        let vertex_mismatch = triangle_primitive(
            None,
            1,
            vec![position_morph_target(0, 2, provenance)],
            provenance,
        );
        assert!(matches!(
            vertex_mismatch,
            Err(MeshPrimitiveError::MorphTargets(
                PrimitiveMorphSetError::VertexCountMismatch {
                    stream: 0,
                    expected: 3,
                    actual: 2,
                }
            ))
        ));
    }

    #[test]
    fn logical_mesh_requires_primitives_and_preserves_mixed_topology_materials() {
        let provenance = provenance();
        assert!(matches!(
            LogicalMesh::try_new(
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                LogicalMeshBounds::default(),
                provenance,
            ),
            Err(LogicalMeshError::NoPrimitives)
        ));

        let triangle =
            triangle_primitive(Some(MaterialIndex::from_u32(0)), 1, Vec::new(), provenance)
                .unwrap();
        let line = MeshPrimitive::try_new(
            2,
            Topology::LineList,
            IndexStream::non_indexed(),
            Some(MaterialIndex::from_u32(1)),
            vec![positions(2, provenance)],
            None,
            Vec::new(),
            1,
            PrimitiveBounds::default(),
            provenance,
        )
        .unwrap();
        let mesh = LogicalMesh::try_new(
            Some(EntityName::try_new("mixed").unwrap()),
            vec![triangle, line],
            vec![
                MorphTargetDefinition::try_new(None, provenance)
                    .expect("test morph definition should be valid"),
            ],
            vec![0.25],
            LogicalMeshBounds::default(),
            provenance,
        )
        .unwrap();

        assert_eq!(mesh.name().map(EntityName::as_str), Some("mixed"));
        assert_eq!(mesh.primitives().len(), 2);
        assert_eq!(
            mesh.primitives().first().unwrap().topology().kind(),
            TopologyKind::TriangleList
        );
        assert_eq!(
            mesh.primitives().get(1).unwrap().topology().kind(),
            TopologyKind::LineList
        );
        assert_eq!(mesh.default_morph_weights(), &[0.25]);
    }

    #[test]
    fn logical_mesh_rejects_invalid_default_morph_weights() {
        let provenance = provenance();
        let primitive = triangle_primitive(None, 1, Vec::new(), provenance).unwrap();
        let count_mismatch = LogicalMesh::try_new(
            None,
            vec![primitive],
            vec![
                MorphTargetDefinition::try_new(None, provenance)
                    .expect("test morph definition should be valid"),
            ],
            Vec::new(),
            LogicalMeshBounds::default(),
            provenance,
        );
        assert!(matches!(
            count_mismatch,
            Err(LogicalMeshError::DefaultMorphWeightCountMismatch {
                expected: 1,
                actual: 0,
            })
        ));

        let primitive = triangle_primitive(None, 1, Vec::new(), provenance).unwrap();
        let non_finite = LogicalMesh::try_new(
            None,
            vec![primitive],
            vec![
                MorphTargetDefinition::try_new(None, provenance)
                    .expect("test morph definition should be valid"),
            ],
            vec![f32::NAN],
            LogicalMeshBounds::default(),
            provenance,
        );
        assert!(matches!(
            non_finite,
            Err(LogicalMeshError::NonFiniteDefaultMorphWeight { index: 0 })
        ));
    }

    #[test]
    fn mesh_instance_stores_only_private_typed_indexes_and_instance_data() {
        let provenance = provenance();
        let instance = MeshInstance::try_new(
            NodeIndex::from_u32(7),
            LogicalMeshIndex::from_u32(2),
            Some(SkinIndex::from_u32(3)),
            2,
            Some(vec![0.25, -0.5]),
            vec![instance_color(1, provenance)],
            provenance,
        )
        .unwrap();

        assert_eq!(instance.owner_index(), NodeIndex::from_u32(7));
        assert_eq!(instance.logical_mesh_index(), LogicalMeshIndex::from_u32(2));
        assert_eq!(instance.skin_index(), Some(SkinIndex::from_u32(3)));
        assert_eq!(
            instance.morph_weight_override(),
            Some([0.25, -0.5].as_slice())
        );
        assert_eq!(instance.domain_count(AttributeDomain::Instance), Some(1));
        assert_eq!(instance.domain_count(AttributeDomain::Vertex), None);

        let debug = format!("{instance:?}");
        assert!(!debug.contains("NodeIndex"));
        assert!(!debug.contains("LogicalMeshIndex"));
        assert!(!debug.contains("SkinIndex"));
        assert!(!debug.contains("0.25"));
        assert!(debug.contains("morph_weight_override_count: Some(2)"));
    }

    #[test]
    fn mesh_instance_rejects_wrong_domain_cardinality_keys_and_overrides() {
        let provenance = provenance();
        let wrong_domain = MeshInstance::try_new(
            NodeIndex::from_u32(0),
            LogicalMeshIndex::from_u32(0),
            None,
            0,
            None,
            vec![positions(1, provenance)],
            provenance,
        );
        assert!(matches!(
            wrong_domain,
            Err(MeshInstanceError::Attributes(
                MeshInstanceAttributeError::InvalidDomain {
                    attribute: 0,
                    domain: AttributeDomain::Vertex,
                }
            ))
        ));

        let wrong_cardinality = MeshInstance::try_new(
            NodeIndex::from_u32(0),
            LogicalMeshIndex::from_u32(0),
            None,
            0,
            None,
            vec![instance_color(2, provenance)],
            provenance,
        );
        assert!(matches!(
            wrong_cardinality,
            Err(MeshInstanceError::Attributes(
                MeshInstanceAttributeError::Cardinality {
                    attribute: 0,
                    actual: 2,
                }
            ))
        ));

        let duplicate = MeshInstance::try_new(
            NodeIndex::from_u32(0),
            LogicalMeshIndex::from_u32(0),
            None,
            0,
            None,
            vec![instance_color(1, provenance), instance_color(1, provenance)],
            provenance,
        );
        assert!(matches!(
            duplicate,
            Err(MeshInstanceError::Attributes(
                MeshInstanceAttributeError::Keys(AttributeKeyValidationError::Duplicate(_))
            ))
        ));

        let wrong_override = MeshInstance::try_new(
            NodeIndex::from_u32(0),
            LogicalMeshIndex::from_u32(0),
            None,
            2,
            Some(vec![0.0]),
            Vec::new(),
            provenance,
        );
        assert!(matches!(
            wrong_override,
            Err(MeshInstanceError::MorphWeightOverride(
                MorphWeightOverrideError::CountMismatch {
                    expected: 2,
                    actual: 1,
                }
            ))
        ));

        let non_finite = MeshInstance::try_new(
            NodeIndex::from_u32(0),
            LogicalMeshIndex::from_u32(0),
            None,
            1,
            Some(vec![f32::INFINITY]),
            Vec::new(),
            provenance,
        );
        assert!(matches!(
            non_finite,
            Err(MeshInstanceError::MorphWeightOverride(
                MorphWeightOverrideError::NonFiniteWeight { index: 0 }
            ))
        ));
    }

    #[test]
    fn owner_typed_bounds_validate_numeric_extent_and_empty_geometry() {
        let generation = SceneGenerationId::allocate().unwrap();
        assert_eq!(
            PrimitiveBounds::try_from_validated(
                Derived::absent(DerivedAbsenceReason::NotProvided),
                MorphBoundsPolicy::BaseGeometry,
            ),
            Err(GeometryBoundsError::MissingValidatedValue)
        );
        let valid = Derived::valid(
            Aabb {
                min: Vec3::new(-1.0, -2.0, -3.0),
                max: Vec3::new(1.0, 2.0, 3.0),
            },
            generation,
            InputStamp::from_bytes([1; 32]),
            DerivedProducer::source(),
        );
        let bounds =
            PrimitiveBounds::try_from_validated(valid, MorphBoundsPolicy::BaseGeometry).unwrap();
        assert!(bounds.derived().is_valid());

        let inverted = Derived::valid(
            Aabb {
                min: Vec3::new(2.0, 0.0, 0.0),
                max: Vec3::new(1.0, 1.0, 1.0),
            },
            generation,
            InputStamp::from_bytes([2; 32]),
            DerivedProducer::source(),
        );
        assert_eq!(
            LogicalMeshBounds::try_from_validated(inverted, MorphBoundsPolicy::DefaultWeights),
            Err(GeometryBoundsError::Inverted {
                axis: BoundsAxis::X,
            })
        );

        let valid_empty = Derived::valid(
            Aabb {
                min: Vec3::ZERO,
                max: Vec3::ZERO,
            },
            generation,
            InputStamp::from_bytes([3; 32]),
            DerivedProducer::source(),
        );
        let empty_bounds =
            PrimitiveBounds::try_from_validated(valid_empty, MorphBoundsPolicy::BaseGeometry)
                .unwrap();
        let empty = MeshPrimitive::try_new(
            0,
            Topology::Points,
            IndexStream::non_indexed(),
            None,
            Vec::new(),
            None,
            Vec::new(),
            0,
            empty_bounds,
            provenance(),
        );
        assert!(matches!(
            empty,
            Err(MeshPrimitiveError::BoundsOnEmptyVertexDomain)
        ));
    }

    #[test]
    fn constructors_compact_hostile_spare_capacity_and_report_retained_storage() {
        let provenance = provenance();

        let mut face_counts = Vec::with_capacity(1_024);
        face_counts.push(3);
        let source_face_bytes = face_counts.capacity() * std::mem::size_of::<u32>();
        let boundaries = PolygonBoundaries::try_new(face_counts).unwrap();
        assert!(boundaries.retained_footprint().unwrap().bytes() < source_face_bytes);

        let mut indices = Vec::with_capacity(1_024);
        indices.extend([0, 1, 2]);
        let source_index_bytes = indices.capacity() * std::mem::size_of::<u32>();
        let index_stream = IndexStream::try_indexed(indices).unwrap();
        assert!(index_stream.retained_footprint().unwrap().bytes() < source_index_bytes);

        let mut attributes = Vec::with_capacity(1_024);
        attributes.push(positions(3, provenance));
        let mut morph_targets = Vec::with_capacity(1_024);
        morph_targets.push(position_morph_target(0, 3, provenance));
        let primitive = MeshPrimitive::try_new(
            3,
            Topology::TriangleList,
            index_stream,
            None,
            attributes,
            None,
            morph_targets,
            1,
            PrimitiveBounds::default(),
            provenance,
        )
        .unwrap();
        assert!(primitive.attributes.capacity() < 1_024);
        assert!(primitive.morph_targets.capacity() < 1_024);
        let primitive_footprint = primitive.retained_footprint().unwrap();
        assert!(
            primitive_footprint.bytes()
                >= primitive.attributes.capacity() * std::mem::size_of::<Attribute>()
        );

        let mut primitives = Vec::with_capacity(1_024);
        primitives.push(primitive);
        let mut definitions = Vec::with_capacity(1_024);
        definitions.push(
            MorphTargetDefinition::try_new(None, provenance)
                .expect("test morph definition should be valid"),
        );
        let mut weights = Vec::with_capacity(1_024);
        weights.push(0.0);
        let mesh = LogicalMesh::try_new(
            None,
            primitives,
            definitions,
            weights,
            LogicalMeshBounds::default(),
            provenance,
        )
        .unwrap();
        assert!(mesh.primitives.capacity() < 1_024);
        assert!(mesh.morph_targets.capacity() < 1_024);
        assert!(mesh.default_morph_weights.capacity() < 1_024);
        assert!(mesh.retained_footprint().unwrap().bytes() >= primitive_footprint.bytes());

        let mut override_weights = Vec::with_capacity(1_024);
        override_weights.push(0.0);
        let mut instance_attributes = Vec::with_capacity(1_024);
        instance_attributes.push(instance_color(1, provenance));
        let instance = MeshInstance::try_new(
            NodeIndex::from_u32(0),
            LogicalMeshIndex::from_u32(0),
            None,
            1,
            Some(override_weights),
            instance_attributes,
            provenance,
        )
        .unwrap();
        assert!(instance.morph_weight_override.as_ref().unwrap().capacity() < 1_024);
        assert!(instance.attributes.capacity() < 1_024);
        assert!(instance.retained_footprint().unwrap().bytes() > 0);
    }

    #[test]
    fn geometry_owners_preserve_and_account_for_multiple_provenance_attachments() {
        let (_arena, [first, second]) = test_provenance_pair_fixture();

        let single_primitive = primitive_with_owner_provenance(first, first);
        let multi_primitive = primitive_with_owner_provenance(
            first,
            ProvenanceAttachments::try_from_iter([first, second]).unwrap(),
        );
        assert_eq!(
            multi_primitive.provenance().iter().collect::<Vec<_>>(),
            [first, second]
        );
        let primitive_attachment_cells = multi_primitive.provenance().retained_allocation_cells();
        let primitive_attachment_bytes = multi_primitive.provenance().retained_allocation_bytes();
        let single_footprint = single_primitive.retained_footprint().unwrap();
        let multi_footprint = multi_primitive.retained_footprint().unwrap();
        assert_eq!(
            multi_footprint.collection_slots() - single_footprint.collection_slots(),
            primitive_attachment_cells
        );
        assert_eq!(
            multi_footprint.bytes() - single_footprint.bytes(),
            primitive_attachment_bytes
        );

        let single_mesh = LogicalMesh::try_new(
            None,
            vec![primitive_with_owner_provenance(first, first)],
            Vec::new(),
            Vec::new(),
            LogicalMeshBounds::default(),
            first,
        )
        .unwrap();
        let multi_mesh = LogicalMesh::try_new(
            None,
            vec![primitive_with_owner_provenance(first, first)],
            Vec::new(),
            Vec::new(),
            LogicalMeshBounds::default(),
            ProvenanceAttachments::try_from_iter([first, second]).unwrap(),
        )
        .unwrap();
        assert_eq!(
            multi_mesh.provenance().iter().collect::<Vec<_>>(),
            [first, second]
        );
        assert_eq!(
            multi_mesh.retained_footprint().unwrap().bytes()
                - single_mesh.retained_footprint().unwrap().bytes(),
            multi_mesh.provenance().retained_allocation_bytes()
        );

        let single_instance = MeshInstance::try_new(
            NodeIndex::from_u32(0),
            LogicalMeshIndex::from_u32(0),
            None,
            0,
            None,
            Vec::new(),
            first,
        )
        .unwrap();
        let multi_instance = MeshInstance::try_new(
            NodeIndex::from_u32(0),
            LogicalMeshIndex::from_u32(0),
            None,
            0,
            None,
            Vec::new(),
            ProvenanceAttachments::try_from_iter([first, second]).unwrap(),
        )
        .unwrap();
        assert_eq!(
            multi_instance.provenance().iter().collect::<Vec<_>>(),
            [first, second]
        );
        assert_eq!(
            multi_instance.retained_footprint().unwrap().bytes()
                - single_instance.retained_footprint().unwrap().bytes(),
            multi_instance.provenance().retained_allocation_bytes()
        );
    }

    #[test]
    fn retained_footprint_overflow_is_transactional() {
        let mut footprint = GeometryRetainedFootprint::default();
        assert_eq!(
            footprint.try_add_capacity::<u16>(usize::MAX, GeometryAllocation::IndexElements),
            Err(GeometryStorageError::RetainedFootprintOverflow {
                resource: GeometryAllocation::IndexElements,
            })
        );
        assert_eq!(footprint, GeometryRetainedFootprint::default());
    }
}
