//! Generation-qualified entity and attribute-domain remap relations.
//!
//! A remap is built from an explicit input/output obligation set. Every input
//! endpoint receives exactly one disposition and every output endpoint receives
//! exactly one origin clause. The representation retains both directions:
//! ordered old-to-new targets for splits and ordered new-to-old origins for
//! merges. Direct construction uses fixed-pass radix canonicalization and is
//! linear in obligations and clause cells. Composition is exposed only through
//! [`RemapSequence`], which left-folds the materialized relation with one direct
//! [`RemapStep`] right-hand side at a time; it is expected-linear in endpoints
//! and materialized relation cells.
//!
//! `SceneEdit` is the integration owner: it retains one [`RemapSequence`] for
//! each remap subject declared by a successful operation plan and publishes
//! those sequences only after final scene validation. This kernel deliberately
//! does not construct an edit report or access scene collections. After a
//! provenance arena fork, `SceneEdit` must remap every derivation attachment and
//! revalidate it against the arena owned by the committed report/scene.

use crate::attribute::AttributeDomain;
use crate::identity::{
    AnimationClipId, CameraId, ImageId, LightId, LogicalMeshId, MaterialId, MeshInstanceId,
    MorphTargetId, NodeId, PrimitiveId, SamplerId, SceneEntityKind, SceneGenerationId, SkinId,
    TextureId,
};
use crate::provenance::{
    OperationIdentity, Provenance, ProvenanceAccessError, ProvenanceArena, ProvenanceForkRemap,
    ProvenanceNodeView, ProvenanceRemapError,
};
use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::marker::PhantomData;
use std::ops::Range;
use thiserror::Error;

/// Absolute endpoint bound for one side of one remap envelope.
pub const MAX_REMAP_ENDPOINTS_PER_SIDE: usize = 4_194_304;

/// Absolute bound for relation or derivation cells in one remap envelope.
pub const MAX_REMAP_RELATION_CELLS: usize = 16_777_216;

/// Absolute number of ordered remap steps retained by one sequence.
pub const MAX_REMAP_STEPS: usize = 4_096;

/// Absolute deterministic work bound for one build or composition sequence.
pub const MAX_REMAP_WORK_UNITS: usize = 134_217_728;

/// A checked operation-node attachment carried by an originless generated endpoint.
///
/// The wrapper is constructible only after resolving the attachment in its
/// owning arena and proving that it addresses an operation node. It stores only
/// the opaque attachment; the potentially large operation identity remains in
/// the scene-owned provenance arena.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DerivationProvenance(Provenance);

impl DerivationProvenance {
    /// Validates that `provenance` belongs to `arena` and addresses an operation.
    pub fn try_new(
        arena: &ProvenanceArena,
        provenance: Provenance,
    ) -> Result<Self, DerivationProvenanceError> {
        match arena.view(&provenance)?.node() {
            ProvenanceNodeView::Source(_) => Err(DerivationProvenanceError::SourceAttachment),
            ProvenanceNodeView::Operation(_) => Ok(Self(provenance)),
        }
    }

    /// Returns the opaque checked attachment.
    pub const fn attachment(self) -> Provenance {
        self.0
    }

    /// Resolves the canonical operation identity in the supplied owning arena.
    pub fn operation_identity<'a>(
        &self,
        arena: &'a ProvenanceArena,
    ) -> Result<&'a OperationIdentity, DerivationProvenanceError> {
        match arena.view(&self.0)?.node() {
            ProvenanceNodeView::Source(_) => Err(DerivationProvenanceError::SourceAttachment),
            ProvenanceNodeView::Operation(operation) => Ok(operation.identity()),
        }
    }

    /// Rebinds this checked operation attachment through an arena fork remap.
    ///
    /// The mapped attachment is revalidated in `target_arena`; translating the
    /// opaque nonce alone is not sufficient evidence that the caller supplied
    /// the immutable arena which will own the published remap report.
    pub fn try_rebind(
        self,
        remap: &ProvenanceForkRemap,
        target_arena: &ProvenanceArena,
    ) -> Result<Self, DerivationProvenanceRebindError> {
        remap.validate_target_arena(target_arena)?;
        let mapped = remap.remap(&self.0)?;
        Self::try_new(target_arena, mapped).map_err(DerivationProvenanceRebindError::from)
    }
}

impl fmt::Debug for DerivationProvenance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("DerivationProvenance")
            .field(&self.0)
            .finish()
    }
}

/// Why a generated derivation attachment was rejected.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum DerivationProvenanceError {
    #[error(transparent)]
    Access(#[from] ProvenanceAccessError),

    #[error("generated derivation provenance must address an operation node")]
    SourceAttachment,
}

/// Why a checked derivation attachment could not follow an arena fork.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum DerivationProvenanceRebindError {
    #[error(transparent)]
    Remap(#[from] ProvenanceRemapError),

    #[error(transparent)]
    Validation(#[from] DerivationProvenanceError),
}

macro_rules! define_primitive_domain_endpoint {
    ($(#[$metadata:meta])* $name:ident) => {
        $(#[$metadata])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name {
            primitive: PrimitiveId,
            domain_index: u32,
        }

        impl $name {
            pub(crate) const fn new(primitive: PrimitiveId, domain_index: u32) -> Self {
                Self {
                    primitive,
                    domain_index,
                }
            }

            /// Returns the generation- and owner-qualified primitive endpoint.
            pub const fn primitive(self) -> PrimitiveId {
                self.primitive
            }

            /// Returns the primitive-local domain index.
            pub const fn domain_index(self) -> u32 {
                self.domain_index
            }

        }
    };
}

define_primitive_domain_endpoint!(
    /// A vertex-domain endpoint identified by exactly `(PrimitiveId, u32)`.
    VertexEndpoint
);
define_primitive_domain_endpoint!(
    /// A corner-domain endpoint identified by exactly `(PrimitiveId, u32)`.
    CornerEndpoint
);
define_primitive_domain_endpoint!(
    /// A face-domain endpoint identified by exactly `(PrimitiveId, u32)`.
    FaceEndpoint
);

mod sealed {
    use super::*;

    pub trait Endpoint: Copy + Eq + Ord + Hash + fmt::Debug {
        const CANONICAL_KEY_BYTES: usize;

        fn generation(self) -> SceneGenerationId;
        fn rebind(self, generation: SceneGenerationId) -> Self;
        fn canonical_key(self) -> u128;
    }

    pub trait Space {}
}

macro_rules! impl_scene_wide_endpoint {
    ($($endpoint:ty),+ $(,)?) => {
        $(
            impl sealed::Endpoint for $endpoint {
                const CANONICAL_KEY_BYTES: usize = std::mem::size_of::<u32>();

                fn generation(self) -> SceneGenerationId {
                    self.generation()
                }

                fn rebind(self, generation: SceneGenerationId) -> Self {
                    self.rebind(generation)
                }

                fn canonical_key(self) -> u128 {
                    self.index() as u128
                }
            }
        )+
    };
}

impl_scene_wide_endpoint!(
    NodeId,
    MeshInstanceId,
    LogicalMeshId,
    MaterialId,
    ImageId,
    SamplerId,
    TextureId,
    SkinId,
    AnimationClipId,
    CameraId,
    LightId,
);

macro_rules! impl_owner_qualified_endpoint {
    ($($endpoint:ty),+ $(,)?) => {
        $(
            impl sealed::Endpoint for $endpoint {
                const CANONICAL_KEY_BYTES: usize = 2 * std::mem::size_of::<u32>();

                fn generation(self) -> SceneGenerationId {
                    self.generation()
                }

                fn rebind(self, generation: SceneGenerationId) -> Self {
                    self.rebind(generation)
                }

                fn canonical_key(self) -> u128 {
                    ((self.logical_mesh().index() as u128) << u32::BITS)
                        | self.local() as u128
                }
            }
        )+
    };
}

impl_owner_qualified_endpoint!(PrimitiveId, MorphTargetId);

macro_rules! impl_domain_endpoint {
    ($($endpoint:ty),+ $(,)?) => {
        $(
            impl sealed::Endpoint for $endpoint {
                const CANONICAL_KEY_BYTES: usize = 3 * std::mem::size_of::<u32>();

                fn generation(self) -> SceneGenerationId {
                    self.primitive.generation()
                }

                fn rebind(self, generation: SceneGenerationId) -> Self {
                    Self {
                        primitive: self.primitive.rebind(generation),
                        domain_index: self.domain_index,
                    }
                }

                fn canonical_key(self) -> u128 {
                    ((self.primitive.logical_mesh().index() as u128) << (2 * u32::BITS))
                        | ((self.primitive.local() as u128) << u32::BITS)
                        | self.domain_index as u128
                }
            }
        )+
    };
}

impl_domain_endpoint!(VertexEndpoint, CornerEndpoint, FaceEndpoint);

/// The exact identity or attribute domain covered by one typed remap space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RemapSubject {
    Entity(SceneEntityKind),
    Domain(AttributeDomain),
}

/// A sealed remap space with one statically selected endpoint type.
///
/// Sealing prevents downstream code from admitting numeric or partially
/// qualified endpoint representations.
pub trait RemapSpace: sealed::Space {
    type Endpoint: sealed::Endpoint;

    const SUBJECT: RemapSubject;
}

macro_rules! define_entity_space {
    ($(#[$metadata:meta])* $space:ident, $endpoint:ty, $kind:ident) => {
        $(#[$metadata])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $space;

        impl sealed::Space for $space {}

        impl RemapSpace for $space {
            type Endpoint = $endpoint;

            const SUBJECT: RemapSubject = RemapSubject::Entity(SceneEntityKind::$kind);
        }
    };
}

define_entity_space!(NodeRemapSpace, NodeId, Node);
define_entity_space!(MeshInstanceRemapSpace, MeshInstanceId, MeshInstance);
define_entity_space!(LogicalMeshRemapSpace, LogicalMeshId, LogicalMesh);
define_entity_space!(PrimitiveRemapSpace, PrimitiveId, Primitive);
define_entity_space!(MorphTargetRemapSpace, MorphTargetId, MorphTarget);
define_entity_space!(MaterialRemapSpace, MaterialId, Material);
define_entity_space!(ImageRemapSpace, ImageId, Image);
define_entity_space!(SamplerRemapSpace, SamplerId, Sampler);
define_entity_space!(TextureRemapSpace, TextureId, Texture);
define_entity_space!(SkinRemapSpace, SkinId, Skin);
define_entity_space!(AnimationClipRemapSpace, AnimationClipId, AnimationClip);
define_entity_space!(CameraRemapSpace, CameraId, Camera);
define_entity_space!(LightRemapSpace, LightId, Light);

macro_rules! define_domain_space {
    ($(#[$metadata:meta])* $space:ident, $endpoint:ty, $domain:ident) => {
        $(#[$metadata])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $space;

        impl sealed::Space for $space {}

        impl RemapSpace for $space {
            type Endpoint = $endpoint;

            const SUBJECT: RemapSubject = RemapSubject::Domain(AttributeDomain::$domain);
        }
    };
}

define_domain_space!(VertexRemapSpace, VertexEndpoint, Vertex);
define_domain_space!(CornerRemapSpace, CornerEndpoint, Corner);
define_domain_space!(FaceRemapSpace, FaceEndpoint, Face);
// ADR 0030 defines the instance-domain endpoint as exactly MeshInstanceId.
define_domain_space!(InstanceRemapSpace, MeshInstanceId, Instance);

/// Which generation boundary side owns an endpoint or obligation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemapSide {
    From,
    To,
}

/// Whether an envelope records structural relations or an explicit identity relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemapChange {
    Changed,
    ExplicitlyUnchanged,
}

/// A zero-allocation identity relation across two scene generations.
///
/// Scene-edit reports use this marker when a remap subject is known to be
/// structurally unchanged. Callers may rebind an endpoint they already proved
/// belongs to the input scene without materializing the complete endpoint
/// universe in the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExplicitUnchangedRemap<S: RemapSpace> {
    from_generation: SceneGenerationId,
    to_generation: SceneGenerationId,
    marker: PhantomData<S>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RemapGenerationBoundary {
    from_generation: SceneGenerationId,
    to_generation: SceneGenerationId,
}

impl RemapGenerationBoundary {
    pub(crate) fn try_new(
        from_generation: SceneGenerationId,
        to_generation: SceneGenerationId,
    ) -> Result<Self, RemapGenerationBoundaryError> {
        if from_generation == to_generation {
            return Err(RemapGenerationBoundaryError::SameGeneration);
        }
        Ok(Self {
            from_generation,
            to_generation,
        })
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemapGenerationBoundaryError {
    #[error("a remap boundary requires distinct scene generations")]
    SameGeneration,
}

impl<S: RemapSpace> ExplicitUnchangedRemap<S> {
    pub(crate) const fn from_boundary(boundary: RemapGenerationBoundary) -> Self {
        Self {
            from_generation: boundary.from_generation,
            to_generation: boundary.to_generation,
            marker: PhantomData,
        }
    }

    pub(crate) const fn input_generation(&self) -> SceneGenerationId {
        self.from_generation
    }

    pub(crate) const fn output_generation(&self) -> SceneGenerationId {
        self.to_generation
    }

    pub(crate) const fn change(&self) -> RemapChange {
        RemapChange::ExplicitlyUnchanged
    }

    pub(crate) fn try_rebind(
        &self,
        endpoint: S::Endpoint,
    ) -> Result<S::Endpoint, RemapError<S::Endpoint>> {
        validate_endpoint_generation(RemapSide::From, endpoint, self.from_generation)?;
        Ok(<S::Endpoint as sealed::Endpoint>::rebind(
            endpoint,
            self.to_generation,
        ))
    }
}

/// A bounded allocation category used by remap errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemapAllocation {
    Obligations,
    ObligationCanonicalization,
    ObligationIndex,
    DispositionFlags,
    Clauses,
    ClauseIndex,
    ClauseEndpoints,
    ClauseUniquenessIndex,
    ForwardHeaders,
    ForwardTargets,
    ReverseHeaders,
    ReverseOrigins,
    UnchangedInputs,
    UnchangedOutputs,
    Derivations,
    IntermediateIndex,
    CompositionDeduplication,
    Steps,
}

/// A bounded resource controlled by [`RemapLimits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemapLimit {
    EndpointsPerSide,
    RelationCells,
    Steps,
    WorkUnits,
}

/// Why requested remap limits were invalid.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum RemapLimitsError {
    #[error("requested {limit:?} limit {requested} exceeds the hard maximum {maximum}")]
    ExceedsHardLimit {
        limit: RemapLimit,
        requested: usize,
        maximum: usize,
    },
}

/// Per-operation remap bounds, themselves capped by process-wide hard limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemapLimits {
    endpoints_per_side: usize,
    relation_cells: usize,
    steps: usize,
    work_units: usize,
}

impl RemapLimits {
    /// Creates limits no greater than the absolute remap bounds.
    pub fn try_new(
        endpoints_per_side: usize,
        relation_cells: usize,
        steps: usize,
        work_units: usize,
    ) -> Result<Self, RemapLimitsError> {
        check_hard_limit(
            RemapLimit::EndpointsPerSide,
            endpoints_per_side,
            MAX_REMAP_ENDPOINTS_PER_SIDE,
        )?;
        check_hard_limit(
            RemapLimit::RelationCells,
            relation_cells,
            MAX_REMAP_RELATION_CELLS,
        )?;
        check_hard_limit(RemapLimit::Steps, steps, MAX_REMAP_STEPS)?;
        check_hard_limit(RemapLimit::WorkUnits, work_units, MAX_REMAP_WORK_UNITS)?;
        Ok(Self {
            endpoints_per_side,
            relation_cells,
            steps,
            work_units,
        })
    }

    pub const fn endpoints_per_side(self) -> usize {
        self.endpoints_per_side
    }

    pub const fn relation_cells(self) -> usize {
        self.relation_cells
    }

    pub const fn steps(self) -> usize {
        self.steps
    }

    pub const fn work_units(self) -> usize {
        self.work_units
    }
}

impl Default for RemapLimits {
    fn default() -> Self {
        Self {
            endpoints_per_side: MAX_REMAP_ENDPOINTS_PER_SIDE,
            relation_cells: MAX_REMAP_RELATION_CELLS,
            steps: MAX_REMAP_STEPS,
            work_units: MAX_REMAP_WORK_UNITS,
        }
    }
}

/// Allocator-retained storage owned by one remap envelope or sequence.
///
/// `cells` counts typed slots across every owned vector, including unused
/// capacity. `bytes` applies each vector's concrete element size to that same
/// capacity. The heterogeneous cell count is useful for structural limits;
/// byte accounting is the authoritative allocator footprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RemapRetainedFootprint {
    cells: usize,
    bytes: usize,
}

impl RemapRetainedFootprint {
    /// Returns all allocator-retained typed slots, including spare capacity.
    pub const fn cells(self) -> usize {
        self.cells
    }

    /// Returns checked allocator-retained bytes across all owned vectors.
    pub const fn bytes(self) -> usize {
        self.bytes
    }

    fn try_add<T, E: fmt::Debug>(&mut self, capacity: usize) -> Result<(), RemapError<E>> {
        let cells = self
            .cells
            .checked_add(capacity)
            .ok_or(RemapError::AccountingOverflow)?;
        let bytes = capacity
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(RemapError::AccountingOverflow)?;
        let bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or(RemapError::AccountingOverflow)?;
        self.cells = cells;
        self.bytes = bytes;
        Ok(())
    }

    fn try_merge<E: fmt::Debug>(self, other: Self) -> Result<Self, RemapError<E>> {
        Ok(Self {
            cells: self
                .cells
                .checked_add(other.cells)
                .ok_or(RemapError::AccountingOverflow)?,
            bytes: self
                .bytes
                .checked_add(other.bytes)
                .ok_or(RemapError::AccountingOverflow)?,
        })
    }
}

fn check_hard_limit(
    limit: RemapLimit,
    requested: usize,
    maximum: usize,
) -> Result<(), RemapLimitsError> {
    if requested > maximum {
        return Err(RemapLimitsError::ExceedsHardLimit {
            limit,
            requested,
            maximum,
        });
    }
    Ok(())
}

/// A typed remap build, lookup, or composition failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RemapError<E: fmt::Debug> {
    #[error("remap generations must be distinct")]
    SameGeneration,

    #[error("{side:?} endpoint belongs to generation {actual:?}, expected {expected:?}")]
    EndpointGenerationMismatch {
        side: RemapSide,
        expected: SceneGenerationId,
        actual: SceneGenerationId,
    },

    #[error("duplicate {side:?} obligation: {endpoint:?}")]
    DuplicateObligation { side: RemapSide, endpoint: E },

    #[error("endpoint is not a declared {side:?} obligation: {endpoint:?}")]
    UnknownObligation { side: RemapSide, endpoint: E },

    #[error("endpoint already has a {side:?} disposition: {endpoint:?}")]
    ConflictingDisposition { side: RemapSide, endpoint: E },

    #[error("one relation clause repeats a {side:?} endpoint: {endpoint:?}")]
    DuplicateClauseEndpoint { side: RemapSide, endpoint: E },

    #[error("declared {side:?} obligation has no disposition: {endpoint:?}")]
    OmittedObligation { side: RemapSide, endpoint: E },

    #[error("a split must contain at least two ordered targets")]
    InvalidSplitCardinality,

    #[error("a merge must contain at least two ordered origins")]
    InvalidMergeCardinality,

    #[error("explicit unchanged remap cannot be combined with structural clauses")]
    ExplicitUnchangedAfterMutation,

    #[error("explicit unchanged remap has different input and output cardinalities")]
    ExplicitUnchangedCardinality,

    #[error("explicit unchanged relation cannot rebind endpoint {endpoint:?}")]
    ExplicitUnchangedEndpointMismatch { endpoint: E },

    #[error("ordered remap sequence is empty")]
    EmptySequence,

    #[error(
        "remap step {step} starts at generation {actual:?}, expected previous output {expected:?}"
    )]
    StepBoundaryMismatch {
        step: usize,
        expected: SceneGenerationId,
        actual: SceneGenerationId,
    },

    #[error("intermediate {side:?} endpoint has no adjacent-step obligation: {endpoint:?}")]
    MissingIntermediateEndpoint { side: RemapSide, endpoint: E },

    #[error("remap {limit:?} limit exceeded; maximum {maximum}")]
    LimitExceeded { limit: RemapLimit, maximum: usize },

    #[error("remap accounting overflowed")]
    AccountingOverflow,

    #[error("remap allocation failed for {resource:?}")]
    AllocationFailed { resource: RemapAllocation },

    #[error("derivation provenance could not be rebound: {source}")]
    DerivationRebind {
        #[source]
        source: DerivationProvenanceRebindError,
    },
}

#[derive(Debug)]
struct WorkCounter {
    used: usize,
    maximum: usize,
}

impl WorkCounter {
    const fn new(maximum: usize) -> Self {
        Self { used: 0, maximum }
    }

    fn spend<E: fmt::Debug>(&mut self, units: usize) -> Result<(), RemapError<E>> {
        let next = self
            .used
            .checked_add(units)
            .ok_or(RemapError::AccountingOverflow)?;
        if next > self.maximum {
            return Err(RemapError::LimitExceeded {
                limit: RemapLimit::WorkUnits,
                maximum: self.maximum,
            });
        }
        self.used = next;
        Ok(())
    }
}

#[derive(Debug)]
enum RelationClause<E> {
    Deleted {
        origin: E,
    },
    Mapped {
        origins: Vec<E>,
        targets: Vec<E>,
    },
    Generated {
        target: E,
        derivation: DerivationProvenance,
    },
}

/// A fallible builder that proves complete remap obligations before publication.
pub struct RemapBuilder<S: RemapSpace> {
    from_generation: SceneGenerationId,
    to_generation: SceneGenerationId,
    from_obligations: Vec<S::Endpoint>,
    to_obligations: Vec<S::Endpoint>,
    from_index: HashMap<S::Endpoint, usize>,
    to_index: HashMap<S::Endpoint, usize>,
    from_seen: Vec<bool>,
    to_seen: Vec<bool>,
    clauses: Vec<RelationClause<S::Endpoint>>,
    relation_cells: usize,
    derivation_cells: usize,
    limits: RemapLimits,
    work: WorkCounter,
}

impl<S: RemapSpace> fmt::Debug for RemapBuilder<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemapBuilder")
            .field("subject", &S::SUBJECT)
            .field("from_generation", &self.from_generation)
            .field("to_generation", &self.to_generation)
            .field("from_obligations", &self.from_obligations.len())
            .field("to_obligations", &self.to_obligations.len())
            .field("clauses", &self.clauses.len())
            .field("relation_cells", &self.relation_cells)
            .field("derivation_cells", &self.derivation_cells)
            .field("work_units", &self.work.used)
            .finish()
    }
}

impl<S: RemapSpace> RemapBuilder<S> {
    /// Creates a builder using the hard default limits.
    pub fn try_new<F, T>(
        from_generation: SceneGenerationId,
        to_generation: SceneGenerationId,
        from_obligations: F,
        to_obligations: T,
    ) -> Result<Self, RemapError<S::Endpoint>>
    where
        F: IntoIterator<Item = S::Endpoint>,
        T: IntoIterator<Item = S::Endpoint>,
    {
        Self::try_new_with_limits(
            from_generation,
            to_generation,
            from_obligations,
            to_obligations,
            RemapLimits::default(),
        )
    }

    /// Creates a builder and canonicalizes both declared endpoint sets.
    pub fn try_new_with_limits<F, T>(
        from_generation: SceneGenerationId,
        to_generation: SceneGenerationId,
        from_obligations: F,
        to_obligations: T,
        limits: RemapLimits,
    ) -> Result<Self, RemapError<S::Endpoint>>
    where
        F: IntoIterator<Item = S::Endpoint>,
        T: IntoIterator<Item = S::Endpoint>,
    {
        if from_generation == to_generation {
            return Err(RemapError::SameGeneration);
        }

        let mut work = WorkCounter::new(limits.work_units);
        let from_obligations = collect_obligations(
            from_obligations,
            RemapSide::From,
            from_generation,
            limits,
            &mut work,
        )?;
        let to_obligations = collect_obligations(
            to_obligations,
            RemapSide::To,
            to_generation,
            limits,
            &mut work,
        )?;
        let from_index = build_endpoint_index(
            from_obligations.iter().copied(),
            RemapAllocation::ObligationIndex,
            &mut work,
        )?;
        let to_index = build_endpoint_index(
            to_obligations.iter().copied(),
            RemapAllocation::ObligationIndex,
            &mut work,
        )?;
        let from_seen = try_false_vec::<S::Endpoint>(from_obligations.len())?;
        let to_seen = try_false_vec::<S::Endpoint>(to_obligations.len())?;

        Ok(Self {
            from_generation,
            to_generation,
            from_obligations,
            to_obligations,
            from_index,
            to_index,
            from_seen,
            to_seen,
            clauses: Vec::new(),
            relation_cells: 0,
            derivation_cells: 0,
            limits,
            work,
        })
    }

    /// Records one deleted input endpoint.
    pub fn try_delete(&mut self, origin: S::Endpoint) -> Result<(), RemapError<S::Endpoint>> {
        let origin_index = self.validate_endpoint(RemapSide::From, origin)?;
        self.ensure_unseen(RemapSide::From, origin, origin_index)?;
        self.reserve_clause()?;
        self.from_seen[origin_index] = true;
        self.clauses.push(RelationClause::Deleted { origin });
        Ok(())
    }

    /// Records one ordinary one-to-one relation whose dense address may change.
    pub fn try_map(
        &mut self,
        origin: S::Endpoint,
        target: S::Endpoint,
    ) -> Result<(), RemapError<S::Endpoint>> {
        let origin_index = self.validate_endpoint(RemapSide::From, origin)?;
        let target_index = self.validate_endpoint(RemapSide::To, target)?;
        self.ensure_unseen(RemapSide::From, origin, origin_index)?;
        self.ensure_unseen(RemapSide::To, target, target_index)?;
        self.reserve_relation_cells(1)?;

        let origins = singleton_vec::<S::Endpoint>(origin)?;
        let targets = singleton_vec::<S::Endpoint>(target)?;
        self.reserve_clause()?;

        self.from_seen[origin_index] = true;
        self.to_seen[target_index] = true;
        self.relation_cells += 1;
        self.clauses
            .push(RelationClause::Mapped { origins, targets });
        Ok(())
    }

    /// Records one old endpoint split into at least two ordered targets.
    pub fn try_split<T>(
        &mut self,
        origin: S::Endpoint,
        targets: T,
    ) -> Result<(), RemapError<S::Endpoint>>
    where
        T: IntoIterator<Item = S::Endpoint>,
    {
        let origin_index = self.validate_endpoint(RemapSide::From, origin)?;
        self.ensure_unseen(RemapSide::From, origin, origin_index)?;
        let targets = collect_clause_endpoints(
            targets,
            RemapSide::To,
            self.to_generation,
            self.limits,
            &mut self.work,
        )?;
        if targets.len() < 2 {
            return Err(RemapError::InvalidSplitCardinality);
        }
        self.validate_clause_endpoints(RemapSide::To, &targets)?;
        self.reserve_relation_cells(targets.len())?;

        let origins = singleton_vec::<S::Endpoint>(origin)?;
        self.reserve_clause()?;

        self.from_seen[origin_index] = true;
        for target in &targets {
            let target_index = self.to_index[target];
            self.to_seen[target_index] = true;
        }
        self.relation_cells += targets.len();
        self.clauses
            .push(RelationClause::Mapped { origins, targets });
        Ok(())
    }

    /// Records at least two ordered old origins merged into one target.
    pub fn try_merge<F>(
        &mut self,
        origins: F,
        target: S::Endpoint,
    ) -> Result<(), RemapError<S::Endpoint>>
    where
        F: IntoIterator<Item = S::Endpoint>,
    {
        let target_index = self.validate_endpoint(RemapSide::To, target)?;
        self.ensure_unseen(RemapSide::To, target, target_index)?;
        let origins = collect_clause_endpoints(
            origins,
            RemapSide::From,
            self.from_generation,
            self.limits,
            &mut self.work,
        )?;
        if origins.len() < 2 {
            return Err(RemapError::InvalidMergeCardinality);
        }
        self.validate_clause_endpoints(RemapSide::From, &origins)?;
        self.reserve_relation_cells(origins.len())?;

        let targets = singleton_vec::<S::Endpoint>(target)?;
        self.reserve_clause()?;

        for origin in &origins {
            let origin_index = self.from_index[origin];
            self.from_seen[origin_index] = true;
        }
        self.to_seen[target_index] = true;
        self.relation_cells += origins.len();
        self.clauses
            .push(RelationClause::Mapped { origins, targets });
        Ok(())
    }

    /// Records an originless output with checked derivation provenance.
    pub fn try_generate(
        &mut self,
        target: S::Endpoint,
        derivation: DerivationProvenance,
    ) -> Result<(), RemapError<S::Endpoint>> {
        let target_index = self.validate_endpoint(RemapSide::To, target)?;
        self.ensure_unseen(RemapSide::To, target, target_index)?;
        self.reserve_derivation_cells(1)?;
        self.reserve_clause()?;
        self.to_seen[target_index] = true;
        self.derivation_cells += 1;
        self.clauses
            .push(RelationClause::Generated { target, derivation });
        Ok(())
    }

    /// Verifies complete changed obligations and freezes the relation.
    pub fn try_finish(mut self) -> Result<RemapStep<S>, RemapError<S::Endpoint>> {
        self.ensure_complete()?;
        build_changed_envelope(self)
    }

    /// Freezes an explicit identity relation across distinct generation tokens.
    pub fn try_finish_explicitly_unchanged(self) -> Result<RemapStep<S>, RemapError<S::Endpoint>> {
        if !self.clauses.is_empty() {
            return Err(RemapError::ExplicitUnchangedAfterMutation);
        }
        build_unchanged_envelope(self)
    }

    fn validate_endpoint(
        &mut self,
        side: RemapSide,
        endpoint: S::Endpoint,
    ) -> Result<usize, RemapError<S::Endpoint>> {
        let expected = match side {
            RemapSide::From => self.from_generation,
            RemapSide::To => self.to_generation,
        };
        validate_endpoint_generation(side, endpoint, expected)?;
        self.work.spend::<S::Endpoint>(1)?;
        let index = match side {
            RemapSide::From => self.from_index.get(&endpoint),
            RemapSide::To => self.to_index.get(&endpoint),
        };
        index
            .copied()
            .ok_or(RemapError::UnknownObligation { side, endpoint })
    }

    fn ensure_unseen(
        &self,
        side: RemapSide,
        endpoint: S::Endpoint,
        index: usize,
    ) -> Result<(), RemapError<S::Endpoint>> {
        let seen = match side {
            RemapSide::From => self.from_seen[index],
            RemapSide::To => self.to_seen[index],
        };
        if seen {
            return Err(RemapError::ConflictingDisposition { side, endpoint });
        }
        Ok(())
    }

    fn validate_clause_endpoints(
        &mut self,
        side: RemapSide,
        endpoints: &[S::Endpoint],
    ) -> Result<(), RemapError<S::Endpoint>> {
        let mut unique = HashMap::new();
        try_reserve_map::<S::Endpoint, (), S::Endpoint>(
            &mut unique,
            endpoints.len(),
            RemapAllocation::ClauseUniquenessIndex,
        )?;
        for endpoint in endpoints {
            let index = self.validate_endpoint(side, *endpoint)?;
            if unique.insert(*endpoint, ()).is_some() {
                return Err(RemapError::DuplicateClauseEndpoint {
                    side,
                    endpoint: *endpoint,
                });
            }
            self.ensure_unseen(side, *endpoint, index)?;
        }
        Ok(())
    }

    fn reserve_clause(&mut self) -> Result<(), RemapError<S::Endpoint>> {
        if self.clauses.len() == self.clauses.capacity() {
            self.clauses
                .try_reserve(1)
                .map_err(|_| RemapError::AllocationFailed {
                    resource: RemapAllocation::Clauses,
                })?;
        }
        self.work.spend::<S::Endpoint>(1)
    }

    fn reserve_relation_cells(&self, additional: usize) -> Result<(), RemapError<S::Endpoint>> {
        let next = self
            .relation_cells
            .checked_add(additional)
            .ok_or(RemapError::AccountingOverflow)?;
        if next > self.limits.relation_cells {
            return Err(RemapError::LimitExceeded {
                limit: RemapLimit::RelationCells,
                maximum: self.limits.relation_cells,
            });
        }
        Ok(())
    }

    fn reserve_derivation_cells(&self, additional: usize) -> Result<(), RemapError<S::Endpoint>> {
        let next = self
            .derivation_cells
            .checked_add(additional)
            .ok_or(RemapError::AccountingOverflow)?;
        if next > self.limits.relation_cells {
            return Err(RemapError::LimitExceeded {
                limit: RemapLimit::RelationCells,
                maximum: self.limits.relation_cells,
            });
        }
        Ok(())
    }

    fn ensure_complete(&mut self) -> Result<(), RemapError<S::Endpoint>> {
        for (index, seen) in self.from_seen.iter().copied().enumerate() {
            self.work.spend::<S::Endpoint>(1)?;
            if !seen {
                return Err(RemapError::OmittedObligation {
                    side: RemapSide::From,
                    endpoint: self.from_obligations[index],
                });
            }
        }
        for (index, seen) in self.to_seen.iter().copied().enumerate() {
            self.work.spend::<S::Endpoint>(1)?;
            if !seen {
                return Err(RemapError::OmittedObligation {
                    side: RemapSide::To,
                    endpoint: self.to_obligations[index],
                });
            }
        }
        Ok(())
    }
}

fn validate_endpoint_generation<E: sealed::Endpoint>(
    side: RemapSide,
    endpoint: E,
    expected: SceneGenerationId,
) -> Result<(), RemapError<E>> {
    let actual = endpoint.generation();
    if actual != expected {
        return Err(RemapError::EndpointGenerationMismatch {
            side,
            expected,
            actual,
        });
    }
    Ok(())
}

fn collect_obligations<E, I>(
    endpoints: I,
    side: RemapSide,
    expected_generation: SceneGenerationId,
    limits: RemapLimits,
    work: &mut WorkCounter,
) -> Result<Vec<E>, RemapError<E>>
where
    E: sealed::Endpoint,
    I: IntoIterator<Item = E>,
{
    let mut endpoints = endpoints.into_iter();
    let (lower, upper) = endpoints.size_hint();
    if lower > limits.endpoints_per_side {
        return Err(RemapError::LimitExceeded {
            limit: RemapLimit::EndpointsPerSide,
            maximum: limits.endpoints_per_side,
        });
    }
    let preflight = upper
        .filter(|upper| *upper <= limits.endpoints_per_side)
        .unwrap_or(lower);
    let mut collected = Vec::new();
    try_reserve_vec::<E, E>(&mut collected, preflight, RemapAllocation::Obligations)?;

    for endpoint in endpoints.by_ref() {
        validate_endpoint_generation(side, endpoint, expected_generation)?;
        if collected.len() == limits.endpoints_per_side {
            return Err(RemapError::LimitExceeded {
                limit: RemapLimit::EndpointsPerSide,
                maximum: limits.endpoints_per_side,
            });
        }
        ensure_push_capacity::<E, E>(&mut collected, RemapAllocation::Obligations)?;
        collected.push(endpoint);
        work.spend::<E>(1)?;
    }

    canonicalize_endpoints(&mut collected, work)?;
    if let Some(endpoint) = collected
        .windows(2)
        .find_map(|pair| (pair[0] == pair[1]).then_some(pair[0]))
    {
        return Err(RemapError::DuplicateObligation { side, endpoint });
    }
    Ok(collected)
}

fn canonicalize_endpoints<E: sealed::Endpoint>(
    endpoints: &mut Vec<E>,
    work: &mut WorkCounter,
) -> Result<(), RemapError<E>> {
    if endpoints.len() < 2 {
        work.spend::<E>(endpoints.len())?;
        return Ok(());
    }

    debug_assert!(E::CANONICAL_KEY_BYTES <= std::mem::size_of::<u128>());
    let mut scratch = Vec::new();
    try_reserve_vec::<E, E>(
        &mut scratch,
        endpoints.len(),
        RemapAllocation::ObligationCanonicalization,
    )?;
    work.spend::<E>(endpoints.len())?;
    scratch.extend_from_slice(endpoints);

    for byte_index in 0..E::CANONICAL_KEY_BYTES {
        work.spend::<E>(endpoints.len())?;
        let shift = byte_index * u8::BITS as usize;
        let mut offsets = [0usize; 256];
        for endpoint in endpoints.iter().copied() {
            let digit = ((endpoint.canonical_key() >> shift) & u8::MAX as u128) as usize;
            offsets[digit] = offsets[digit]
                .checked_add(1)
                .ok_or(RemapError::AccountingOverflow)?;
        }

        let mut next_offset = 0usize;
        for offset in &mut offsets {
            let count = *offset;
            *offset = next_offset;
            next_offset = next_offset
                .checked_add(count)
                .ok_or(RemapError::AccountingOverflow)?;
        }
        debug_assert_eq!(next_offset, endpoints.len());

        for endpoint in endpoints.iter().copied() {
            let digit = ((endpoint.canonical_key() >> shift) & u8::MAX as u128) as usize;
            let destination = offsets[digit];
            let slot = scratch
                .get_mut(destination)
                .ok_or(RemapError::AccountingOverflow)?;
            *slot = endpoint;
            offsets[digit] = destination
                .checked_add(1)
                .ok_or(RemapError::AccountingOverflow)?;
        }
        std::mem::swap(endpoints, &mut scratch);
    }

    Ok(())
}

fn collect_clause_endpoints<E, I>(
    endpoints: I,
    side: RemapSide,
    expected_generation: SceneGenerationId,
    limits: RemapLimits,
    work: &mut WorkCounter,
) -> Result<Vec<E>, RemapError<E>>
where
    E: sealed::Endpoint,
    I: IntoIterator<Item = E>,
{
    let mut endpoints = endpoints.into_iter();
    let (lower, upper) = endpoints.size_hint();
    if lower > limits.endpoints_per_side {
        return Err(RemapError::LimitExceeded {
            limit: RemapLimit::EndpointsPerSide,
            maximum: limits.endpoints_per_side,
        });
    }
    let preflight = upper
        .filter(|upper| *upper <= limits.endpoints_per_side)
        .unwrap_or(lower);
    let mut collected = Vec::new();
    try_reserve_vec::<E, E>(&mut collected, preflight, RemapAllocation::ClauseEndpoints)?;
    for endpoint in endpoints.by_ref() {
        validate_endpoint_generation(side, endpoint, expected_generation)?;
        if collected.len() == limits.endpoints_per_side {
            return Err(RemapError::LimitExceeded {
                limit: RemapLimit::EndpointsPerSide,
                maximum: limits.endpoints_per_side,
            });
        }
        ensure_push_capacity::<E, E>(&mut collected, RemapAllocation::ClauseEndpoints)?;
        collected.push(endpoint);
        work.spend::<E>(1)?;
    }
    Ok(collected)
}

fn build_endpoint_index<E, I>(
    endpoints: I,
    resource: RemapAllocation,
    work: &mut WorkCounter,
) -> Result<HashMap<E, usize>, RemapError<E>>
where
    E: sealed::Endpoint,
    I: ExactSizeIterator<Item = E>,
{
    let mut index = HashMap::new();
    try_reserve_map::<E, usize, E>(&mut index, endpoints.len(), resource)?;
    work.spend::<E>(endpoints.len())?;
    for (position, endpoint) in endpoints.enumerate() {
        index.insert(endpoint, position);
    }
    Ok(index)
}

fn try_false_vec<E: fmt::Debug>(length: usize) -> Result<Vec<bool>, RemapError<E>> {
    let mut values = Vec::new();
    try_reserve_vec::<bool, E>(&mut values, length, RemapAllocation::DispositionFlags)?;
    values.resize(length, false);
    Ok(values)
}

fn singleton_vec<E: sealed::Endpoint>(value: E) -> Result<Vec<E>, RemapError<E>> {
    let mut values = Vec::new();
    try_reserve_vec::<E, E>(&mut values, 1, RemapAllocation::ClauseEndpoints)?;
    values.push(value);
    Ok(values)
}

fn try_reserve_vec<T, E: fmt::Debug>(
    values: &mut Vec<T>,
    additional: usize,
    resource: RemapAllocation,
) -> Result<(), RemapError<E>> {
    if additional == 0 {
        return Ok(());
    }
    values
        .try_reserve_exact(additional)
        .map_err(|_| RemapError::AllocationFailed { resource })
}

fn ensure_push_capacity<T, E: fmt::Debug>(
    values: &mut Vec<T>,
    resource: RemapAllocation,
) -> Result<(), RemapError<E>> {
    if values.len() == values.capacity() {
        values
            .try_reserve(1)
            .map_err(|_| RemapError::AllocationFailed { resource })?;
    }
    Ok(())
}

fn try_reserve_map<K, V, E: fmt::Debug>(
    values: &mut HashMap<K, V>,
    additional: usize,
    resource: RemapAllocation,
) -> Result<(), RemapError<E>>
where
    K: Eq + Hash,
{
    if additional == 0 {
        return Ok(());
    }
    values
        .try_reserve(additional)
        .map_err(|_| RemapError::AllocationFailed { resource })
}

#[derive(Debug)]
struct ForwardHeader<E> {
    endpoint: E,
    targets: Range<usize>,
}

#[derive(Debug)]
struct ReverseHeader<E> {
    endpoint: E,
    origins: Range<usize>,
    derivations: Range<usize>,
}

#[derive(Debug)]
enum RemapRelation<E> {
    General {
        forward: Vec<ForwardHeader<E>>,
        forward_targets: Vec<E>,
        reverse: Vec<ReverseHeader<E>>,
        reverse_origins: Vec<E>,
    },
    ExplicitlyUnchanged {
        inputs: Vec<E>,
        outputs: Vec<E>,
    },
}

/// One complete generation-qualified relation for one typed remap space.
///
/// The type intentionally does not implement `Clone`: reports retain large
/// relation payloads by ownership, while composition copies only compact typed
/// endpoints and provenance attachments through fallible buffers.
pub struct RemapEnvelope<S: RemapSpace> {
    from_generation: SceneGenerationId,
    to_generation: SceneGenerationId,
    relation: RemapRelation<S::Endpoint>,
    derivations: Vec<DerivationProvenance>,
    work_units: usize,
    derivation_rebind_work_units: usize,
    marker: PhantomData<S>,
}

/// One direct operation step built from disjoint map, split, merge, delete, and
/// generated clauses.
///
/// Only [`RemapBuilder`] can create this type. Keeping direct steps distinct
/// from composed relations guarantees that sequence composition always adds
/// one endpoint-disjoint right-hand step.
pub struct RemapStep<S: RemapSpace> {
    envelope: RemapEnvelope<S>,
}

/// A materialized input-to-current relation produced only by left-folding
/// direct [`RemapStep`] values.
///
/// This type has no public composition API, so two already-composed relations
/// cannot be multiplied and trigger superlinear duplicate path enumeration.
pub struct ComposedRemap<S: RemapSpace> {
    envelope: RemapEnvelope<S>,
}

impl<S: RemapSpace> std::ops::Deref for RemapStep<S> {
    type Target = RemapEnvelope<S>;

    fn deref(&self) -> &Self::Target {
        &self.envelope
    }
}

impl<S: RemapSpace> std::ops::Deref for ComposedRemap<S> {
    type Target = RemapEnvelope<S>;

    fn deref(&self) -> &Self::Target {
        &self.envelope
    }
}

impl<S: RemapSpace> fmt::Debug for RemapStep<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RemapStep")
            .field(&self.envelope)
            .finish()
    }
}

impl<S: RemapSpace> fmt::Debug for ComposedRemap<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ComposedRemap")
            .field(&self.envelope)
            .finish()
    }
}

impl<S: RemapSpace> RemapStep<S> {
    /// Rebinds every direct-step derivation using the default hard limits.
    pub fn try_rebind_derivations(
        self,
        remap: &ProvenanceForkRemap,
        target_arena: &ProvenanceArena,
    ) -> Result<Self, RemapError<S::Endpoint>> {
        self.try_rebind_derivations_with_limits(remap, target_arena, RemapLimits::default())
    }

    /// Rebinds every direct-step derivation under explicit limits.
    pub fn try_rebind_derivations_with_limits(
        self,
        remap: &ProvenanceForkRemap,
        target_arena: &ProvenanceArena,
        limits: RemapLimits,
    ) -> Result<Self, RemapError<S::Endpoint>> {
        Ok(Self {
            envelope: self.envelope.try_rebind_derivations_with_limits(
                remap,
                target_arena,
                limits,
            )?,
        })
    }
}

impl<S: RemapSpace> fmt::Debug for RemapEnvelope<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemapEnvelope")
            .field("subject", &S::SUBJECT)
            .field("from_generation", &self.from_generation)
            .field("to_generation", &self.to_generation)
            .field("change", &self.change())
            .field("from_endpoints", &self.input_endpoint_count())
            .field("to_endpoints", &self.output_endpoint_count())
            .field("relation_cells", &self.relation_cell_count())
            .field("derivations", &self.derivations.len())
            .field("work_units", &self.work_units)
            .field(
                "derivation_rebind_work_units",
                &self.derivation_rebind_work_units,
            )
            .finish()
    }
}

impl<S: RemapSpace> RemapEnvelope<S> {
    pub const fn subject(&self) -> RemapSubject {
        S::SUBJECT
    }

    pub const fn input_generation(&self) -> SceneGenerationId {
        self.from_generation
    }

    pub const fn output_generation(&self) -> SceneGenerationId {
        self.to_generation
    }

    pub const fn change(&self) -> RemapChange {
        match &self.relation {
            RemapRelation::General { .. } => RemapChange::Changed,
            RemapRelation::ExplicitlyUnchanged { .. } => RemapChange::ExplicitlyUnchanged,
        }
    }

    pub const fn is_explicitly_unchanged(&self) -> bool {
        matches!(&self.relation, RemapRelation::ExplicitlyUnchanged { .. })
    }

    pub fn input_endpoint_count(&self) -> usize {
        match &self.relation {
            RemapRelation::General { forward, .. } => forward.len(),
            RemapRelation::ExplicitlyUnchanged { inputs, .. } => inputs.len(),
        }
    }

    pub fn output_endpoint_count(&self) -> usize {
        match &self.relation {
            RemapRelation::General { reverse, .. } => reverse.len(),
            RemapRelation::ExplicitlyUnchanged { outputs, .. } => outputs.len(),
        }
    }

    pub fn relation_cell_count(&self) -> usize {
        match &self.relation {
            RemapRelation::General {
                forward_targets, ..
            } => forward_targets.len(),
            RemapRelation::ExplicitlyUnchanged { inputs, .. } => inputs.len(),
        }
    }

    pub fn derivation_count(&self) -> usize {
        self.derivations.len()
    }

    pub const fn work_units(&self) -> usize {
        self.work_units
    }

    /// Returns cumulative work spent rebinding retained derivation attachments.
    pub const fn derivation_rebind_work_units(&self) -> usize {
        self.derivation_rebind_work_units
    }

    /// Measures all allocator-retained vector capacity owned by this envelope.
    pub fn retained_footprint(&self) -> Result<RemapRetainedFootprint, RemapError<S::Endpoint>> {
        let mut footprint = RemapRetainedFootprint::default();
        match &self.relation {
            RemapRelation::General {
                forward,
                forward_targets,
                reverse,
                reverse_origins,
            } => {
                footprint.try_add::<ForwardHeader<S::Endpoint>, S::Endpoint>(forward.capacity())?;
                footprint.try_add::<S::Endpoint, S::Endpoint>(forward_targets.capacity())?;
                footprint.try_add::<ReverseHeader<S::Endpoint>, S::Endpoint>(reverse.capacity())?;
                footprint.try_add::<S::Endpoint, S::Endpoint>(reverse_origins.capacity())?;
            }
            RemapRelation::ExplicitlyUnchanged { inputs, outputs } => {
                footprint.try_add::<S::Endpoint, S::Endpoint>(inputs.capacity())?;
                footprint.try_add::<S::Endpoint, S::Endpoint>(outputs.capacity())?;
            }
        }
        footprint.try_add::<DerivationProvenance, S::Endpoint>(self.derivations.capacity())?;
        Ok(footprint)
    }

    /// Returns allocator-retained typed slots, including unused capacity.
    pub fn retained_allocation_cells(&self) -> Result<usize, RemapError<S::Endpoint>> {
        Ok(self.retained_footprint()?.cells())
    }

    /// Returns checked allocator-retained bytes, including unused capacity.
    pub fn retained_allocation_bytes(&self) -> Result<usize, RemapError<S::Endpoint>> {
        Ok(self.retained_footprint()?.bytes())
    }

    /// Rebinds every derivation attachment after complete prevalidation.
    fn try_rebind_derivations_with_limits(
        mut self,
        remap: &ProvenanceForkRemap,
        target_arena: &ProvenanceArena,
        limits: RemapLimits,
    ) -> Result<Self, RemapError<S::Endpoint>> {
        check_composition_envelope_limits(&self, limits)?;
        let required_work = derivation_rebind_work::<S::Endpoint>(self.derivations.len())?;
        let mut work = WorkCounter::new(limits.work_units);
        work.spend::<S::Endpoint>(required_work)?;
        let cumulative_work = self
            .derivation_rebind_work_units
            .checked_add(required_work)
            .ok_or(RemapError::AccountingOverflow)?;

        preflight_derivation_rebind::<S::Endpoint>(&self.derivations, remap, target_arena)?;
        apply_derivation_rebind::<S::Endpoint>(&mut self.derivations, remap, target_arena)?;
        self.derivation_rebind_work_units = cumulative_work;
        Ok(self)
    }

    /// Resolves one input endpoint after checking the envelope generation first.
    pub fn outcome(
        &self,
        endpoint: S::Endpoint,
    ) -> Result<OriginMappingView<'_, S::Endpoint>, RemapError<S::Endpoint>> {
        validate_endpoint_generation(RemapSide::From, endpoint, self.from_generation)?;
        let index = match &self.relation {
            RemapRelation::General { forward, .. } => {
                forward.binary_search_by_key(&endpoint, |header| header.endpoint)
            }
            RemapRelation::ExplicitlyUnchanged { inputs, .. } => inputs.binary_search(&endpoint),
        }
        .map_err(|_| RemapError::UnknownObligation {
            side: RemapSide::From,
            endpoint,
        })?;
        Ok(self.origin_view(index))
    }

    /// Resolves one output endpoint after checking the envelope generation first.
    pub fn origins(
        &self,
        endpoint: S::Endpoint,
    ) -> Result<TargetMappingView<'_, S::Endpoint>, RemapError<S::Endpoint>> {
        validate_endpoint_generation(RemapSide::To, endpoint, self.to_generation)?;
        let index = match &self.relation {
            RemapRelation::General { reverse, .. } => {
                reverse.binary_search_by_key(&endpoint, |header| header.endpoint)
            }
            RemapRelation::ExplicitlyUnchanged { outputs, .. } => outputs.binary_search(&endpoint),
        }
        .map_err(|_| RemapError::UnknownObligation {
            side: RemapSide::To,
            endpoint,
        })?;
        Ok(self.target_view(index))
    }

    /// Iterates input dispositions in canonical endpoint order.
    pub fn outcomes(&self) -> impl ExactSizeIterator<Item = OriginMappingView<'_, S::Endpoint>> {
        (0..self.input_endpoint_count()).map(|index| self.origin_view(index))
    }

    /// Iterates output origins in canonical endpoint order.
    pub fn target_origins(
        &self,
    ) -> impl ExactSizeIterator<Item = TargetMappingView<'_, S::Endpoint>> {
        (0..self.output_endpoint_count()).map(|index| self.target_view(index))
    }

    fn origin_view(&self, index: usize) -> OriginMappingView<'_, S::Endpoint> {
        match &self.relation {
            RemapRelation::General {
                forward,
                forward_targets,
                ..
            } => {
                let header = &forward[index];
                OriginMappingView {
                    endpoint: header.endpoint,
                    targets: &forward_targets[header.targets.clone()],
                }
            }
            RemapRelation::ExplicitlyUnchanged { inputs, outputs } => OriginMappingView {
                endpoint: inputs[index],
                targets: &outputs[index..index + 1],
            },
        }
    }

    fn target_view(&self, index: usize) -> TargetMappingView<'_, S::Endpoint> {
        match &self.relation {
            RemapRelation::General {
                reverse,
                reverse_origins,
                ..
            } => {
                let header = &reverse[index];
                TargetMappingView {
                    endpoint: header.endpoint,
                    origins: &reverse_origins[header.origins.clone()],
                    derivations: &self.derivations[header.derivations.clone()],
                }
            }
            RemapRelation::ExplicitlyUnchanged { inputs, outputs } => TargetMappingView {
                endpoint: outputs[index],
                origins: &inputs[index..index + 1],
                derivations: &self.derivations[0..0],
            },
        }
    }
}

/// Read-only old-to-new disposition for one input endpoint.
#[derive(Debug, Clone, Copy)]
pub struct OriginMappingView<'a, E> {
    endpoint: E,
    targets: &'a [E],
}

impl<'a, E: Copy> OriginMappingView<'a, E> {
    pub const fn endpoint(self) -> E {
        self.endpoint
    }

    pub const fn targets(self) -> &'a [E] {
        self.targets
    }

    pub const fn is_deleted(self) -> bool {
        self.targets.is_empty()
    }
}

/// Read-only new-to-old origins and originless derivations for one output endpoint.
#[derive(Debug, Clone, Copy)]
pub struct TargetMappingView<'a, E> {
    endpoint: E,
    origins: &'a [E],
    derivations: &'a [DerivationProvenance],
}

impl<'a, E: Copy> TargetMappingView<'a, E> {
    pub const fn endpoint(self) -> E {
        self.endpoint
    }

    pub const fn origins(self) -> &'a [E] {
        self.origins
    }

    pub const fn derivations(self) -> &'a [DerivationProvenance] {
        self.derivations
    }

    pub const fn is_originless(self) -> bool {
        self.origins.is_empty()
    }
}

fn build_changed_envelope<S: RemapSpace>(
    mut builder: RemapBuilder<S>,
) -> Result<RemapStep<S>, RemapError<S::Endpoint>> {
    let mut forward = Vec::new();
    let mut forward_targets = Vec::new();
    let mut reverse = Vec::new();
    let mut reverse_origins = Vec::new();
    let mut derivations = Vec::new();
    try_reserve_vec::<_, S::Endpoint>(
        &mut forward,
        builder.from_obligations.len(),
        RemapAllocation::ForwardHeaders,
    )?;
    try_reserve_vec::<_, S::Endpoint>(
        &mut forward_targets,
        builder.relation_cells,
        RemapAllocation::ForwardTargets,
    )?;
    try_reserve_vec::<_, S::Endpoint>(
        &mut reverse,
        builder.to_obligations.len(),
        RemapAllocation::ReverseHeaders,
    )?;
    try_reserve_vec::<_, S::Endpoint>(
        &mut reverse_origins,
        builder.relation_cells,
        RemapAllocation::ReverseOrigins,
    )?;
    try_reserve_vec::<_, S::Endpoint>(
        &mut derivations,
        builder.derivation_cells,
        RemapAllocation::Derivations,
    )?;

    let clause_indexes = index_relation_clauses(&mut builder)?;
    for (ordinal, origin) in builder.from_obligations.iter().copied().enumerate() {
        let clause_index = clause_indexes
            .by_from_obligation
            .get(ordinal)
            .copied()
            .ok_or(RemapError::OmittedObligation {
                side: RemapSide::From,
                endpoint: origin,
            })?;
        let clause = builder
            .clauses
            .get(clause_index)
            .ok_or(RemapError::OmittedObligation {
                side: RemapSide::From,
                endpoint: origin,
            })?;
        let start = forward_targets.len();
        match clause {
            RelationClause::Deleted { .. } => {}
            RelationClause::Mapped { targets, .. } => {
                forward_targets.extend(targets.iter().copied());
                builder.work.spend::<S::Endpoint>(targets.len())?;
            }
            RelationClause::Generated { .. } => {
                return Err(RemapError::OmittedObligation {
                    side: RemapSide::From,
                    endpoint: origin,
                });
            }
        }
        forward.push(ForwardHeader {
            endpoint: origin,
            targets: start..forward_targets.len(),
        });
        builder.work.spend::<S::Endpoint>(1)?;
    }

    for (ordinal, target) in builder.to_obligations.iter().copied().enumerate() {
        let clause_index = clause_indexes
            .by_to_obligation
            .get(ordinal)
            .copied()
            .ok_or(RemapError::OmittedObligation {
                side: RemapSide::To,
                endpoint: target,
            })?;
        let clause = builder
            .clauses
            .get(clause_index)
            .ok_or(RemapError::OmittedObligation {
                side: RemapSide::To,
                endpoint: target,
            })?;
        let origin_start = reverse_origins.len();
        let derivation_start = derivations.len();
        match clause {
            RelationClause::Deleted { .. } => {
                return Err(RemapError::OmittedObligation {
                    side: RemapSide::To,
                    endpoint: target,
                });
            }
            RelationClause::Mapped { origins, .. } => {
                reverse_origins.extend(origins.iter().copied());
                builder.work.spend::<S::Endpoint>(origins.len())?;
            }
            RelationClause::Generated { derivation, .. } => {
                derivations.push(*derivation);
                builder.work.spend::<S::Endpoint>(1)?;
            }
        }
        reverse.push(ReverseHeader {
            endpoint: target,
            origins: origin_start..reverse_origins.len(),
            derivations: derivation_start..derivations.len(),
        });
        builder.work.spend::<S::Endpoint>(1)?;
    }

    Ok(RemapStep {
        envelope: RemapEnvelope {
            from_generation: builder.from_generation,
            to_generation: builder.to_generation,
            relation: RemapRelation::General {
                forward,
                forward_targets,
                reverse,
                reverse_origins,
            },
            derivations,
            work_units: builder.work.used,
            derivation_rebind_work_units: 0,
            marker: PhantomData,
        },
    })
}

struct RelationClauseIndexes {
    by_from_obligation: Vec<usize>,
    by_to_obligation: Vec<usize>,
}

fn index_relation_clauses<S: RemapSpace>(
    builder: &mut RemapBuilder<S>,
) -> Result<RelationClauseIndexes, RemapError<S::Endpoint>> {
    let mut from_clause_indexes =
        try_unassigned_clause_indexes::<S::Endpoint>(builder.from_obligations.len())?;
    let mut to_clause_indexes =
        try_unassigned_clause_indexes::<S::Endpoint>(builder.to_obligations.len())?;

    for (clause_index, clause) in builder.clauses.iter().enumerate() {
        builder.work.spend::<S::Endpoint>(1)?;
        match clause {
            RelationClause::Deleted { origin } => assign_clause_index(
                *origin,
                clause_index,
                RemapSide::From,
                &builder.from_index,
                &mut from_clause_indexes,
                &mut builder.work,
            )?,
            RelationClause::Mapped { origins, targets } => {
                for origin in origins {
                    assign_clause_index(
                        *origin,
                        clause_index,
                        RemapSide::From,
                        &builder.from_index,
                        &mut from_clause_indexes,
                        &mut builder.work,
                    )?;
                }
                for target in targets {
                    assign_clause_index(
                        *target,
                        clause_index,
                        RemapSide::To,
                        &builder.to_index,
                        &mut to_clause_indexes,
                        &mut builder.work,
                    )?;
                }
            }
            RelationClause::Generated { target, .. } => assign_clause_index(
                *target,
                clause_index,
                RemapSide::To,
                &builder.to_index,
                &mut to_clause_indexes,
                &mut builder.work,
            )?,
        }
    }

    Ok(RelationClauseIndexes {
        by_from_obligation: from_clause_indexes,
        by_to_obligation: to_clause_indexes,
    })
}

fn try_unassigned_clause_indexes<E: fmt::Debug>(
    length: usize,
) -> Result<Vec<usize>, RemapError<E>> {
    let mut indexes = Vec::new();
    try_reserve_vec::<usize, E>(&mut indexes, length, RemapAllocation::ClauseIndex)?;
    indexes.resize(length, usize::MAX);
    Ok(indexes)
}

fn assign_clause_index<E: sealed::Endpoint>(
    endpoint: E,
    clause_index: usize,
    side: RemapSide,
    endpoint_index: &HashMap<E, usize>,
    clause_indexes: &mut [usize],
    work: &mut WorkCounter,
) -> Result<(), RemapError<E>> {
    let ordinal = endpoint_index
        .get(&endpoint)
        .copied()
        .ok_or(RemapError::UnknownObligation { side, endpoint })?;
    let assigned = clause_indexes
        .get_mut(ordinal)
        .ok_or(RemapError::UnknownObligation { side, endpoint })?;
    if *assigned != usize::MAX {
        return Err(RemapError::ConflictingDisposition { side, endpoint });
    }
    *assigned = clause_index;
    work.spend::<E>(1)
}

fn build_unchanged_envelope<S: RemapSpace>(
    mut builder: RemapBuilder<S>,
) -> Result<RemapStep<S>, RemapError<S::Endpoint>> {
    if builder.from_obligations.len() != builder.to_obligations.len() {
        return Err(RemapError::ExplicitUnchangedCardinality);
    }

    let count = builder.from_obligations.len();
    if count > builder.limits.relation_cells {
        return Err(RemapError::LimitExceeded {
            limit: RemapLimit::RelationCells,
            maximum: builder.limits.relation_cells,
        });
    }
    for (origin, expected_target) in builder.from_obligations.iter().zip(&builder.to_obligations) {
        validate_explicitly_unchanged_pair::<S>(
            *origin,
            *expected_target,
            builder.from_generation,
            builder.to_generation,
        )?;
        builder.work.spend::<S::Endpoint>(1)?;
    }

    Ok(RemapStep {
        envelope: RemapEnvelope {
            from_generation: builder.from_generation,
            to_generation: builder.to_generation,
            relation: RemapRelation::ExplicitlyUnchanged {
                inputs: builder.from_obligations,
                outputs: builder.to_obligations,
            },
            derivations: Vec::new(),
            work_units: builder.work.used,
            derivation_rebind_work_units: 0,
            marker: PhantomData,
        },
    })
}

fn validate_explicitly_unchanged_pair<S: RemapSpace>(
    origin: S::Endpoint,
    expected_target: S::Endpoint,
    from_generation: SceneGenerationId,
    to_generation: SceneGenerationId,
) -> Result<(), RemapError<S::Endpoint>> {
    validate_endpoint_generation(RemapSide::From, origin, from_generation)?;
    validate_endpoint_generation(RemapSide::To, expected_target, to_generation)?;
    let target = <S::Endpoint as sealed::Endpoint>::rebind(origin, to_generation);
    if target != expected_target {
        return Err(RemapError::ExplicitUnchangedEndpointMismatch { endpoint: target });
    }
    Ok(())
}

fn compose_explicitly_unchanged_pair<S: RemapSpace>(
    left: &RemapEnvelope<S>,
    right: &RemapEnvelope<S>,
    work: &mut WorkCounter,
) -> Result<ComposedRemap<S>, RemapError<S::Endpoint>> {
    let RemapRelation::ExplicitlyUnchanged {
        inputs: left_inputs,
        outputs: left_outputs,
    } = &left.relation
    else {
        return Err(RemapError::AccountingOverflow);
    };
    let RemapRelation::ExplicitlyUnchanged {
        inputs: right_inputs,
        outputs: right_outputs,
    } = &right.relation
    else {
        return Err(RemapError::AccountingOverflow);
    };

    let mut right_index = 0;
    let mut first_missing_from = None;
    for &left_endpoint in left_outputs {
        while let Some(&right_endpoint) = right_inputs.get(right_index)
            && right_endpoint < left_endpoint
        {
            work.spend::<S::Endpoint>(1)?;
            first_missing_from.get_or_insert(right_endpoint);
            right_index += 1;
        }
        work.spend::<S::Endpoint>(1)?;
        match right_inputs.get(right_index) {
            Some(&right_endpoint) if right_endpoint == left_endpoint => {
                right_index += 1;
            }
            Some(_) | None => {
                return Err(RemapError::MissingIntermediateEndpoint {
                    side: RemapSide::To,
                    endpoint: left_endpoint,
                });
            }
        }
    }
    if let Some(endpoint) = first_missing_from {
        return Err(RemapError::MissingIntermediateEndpoint {
            side: RemapSide::From,
            endpoint,
        });
    }
    if let Some(&endpoint) = right_inputs.get(right_index) {
        return Err(RemapError::MissingIntermediateEndpoint {
            side: RemapSide::From,
            endpoint,
        });
    }

    let mut inputs = Vec::new();
    try_reserve_vec::<S::Endpoint, S::Endpoint>(
        &mut inputs,
        left_inputs.len(),
        RemapAllocation::UnchangedInputs,
    )?;
    work.spend::<S::Endpoint>(left_inputs.len())?;
    inputs.extend_from_slice(left_inputs);
    let mut outputs = Vec::new();
    try_reserve_vec::<S::Endpoint, S::Endpoint>(
        &mut outputs,
        right_outputs.len(),
        RemapAllocation::UnchangedOutputs,
    )?;
    work.spend::<S::Endpoint>(right_outputs.len())?;
    outputs.extend_from_slice(right_outputs);

    Ok(ComposedRemap {
        envelope: RemapEnvelope {
            from_generation: left.from_generation,
            to_generation: right.to_generation,
            relation: RemapRelation::ExplicitlyUnchanged { inputs, outputs },
            derivations: Vec::new(),
            work_units: work.used,
            derivation_rebind_work_units: 0,
            marker: PhantomData,
        },
    })
}

fn compose_pair<S: RemapSpace>(
    left: &RemapEnvelope<S>,
    right: &RemapStep<S>,
    limits: RemapLimits,
    work: &mut WorkCounter,
) -> Result<ComposedRemap<S>, RemapError<S::Endpoint>> {
    let right = &right.envelope;
    if left.to_generation != right.from_generation {
        return Err(RemapError::StepBoundaryMismatch {
            step: 1,
            expected: left.to_generation,
            actual: right.from_generation,
        });
    }
    check_composition_envelope_limits(left, limits)?;
    check_composition_envelope_limits(right, limits)?;

    if matches!(&left.relation, RemapRelation::ExplicitlyUnchanged { .. })
        && matches!(&right.relation, RemapRelation::ExplicitlyUnchanged { .. })
    {
        return compose_explicitly_unchanged_pair(left, right, work);
    }

    let left_reverse_index = build_endpoint_index(
        left.target_origins().map(TargetMappingView::endpoint),
        RemapAllocation::IntermediateIndex,
        work,
    )?;
    let right_forward_index = build_endpoint_index(
        right.outcomes().map(OriginMappingView::endpoint),
        RemapAllocation::IntermediateIndex,
        work,
    )?;

    // Validate complete adjacent-step coverage before following a relation.
    for mapping in left.target_origins() {
        work.spend::<S::Endpoint>(1)?;
        if !right_forward_index.contains_key(&mapping.endpoint()) {
            return Err(RemapError::MissingIntermediateEndpoint {
                side: RemapSide::To,
                endpoint: mapping.endpoint(),
            });
        }
    }
    for mapping in right.outcomes() {
        work.spend::<S::Endpoint>(1)?;
        if !left_reverse_index.contains_key(&mapping.endpoint()) {
            return Err(RemapError::MissingIntermediateEndpoint {
                side: RemapSide::From,
                endpoint: mapping.endpoint(),
            });
        }
    }

    let mut forward = Vec::new();
    let mut forward_targets = Vec::new();
    let mut reverse = Vec::new();
    let mut reverse_origins = Vec::new();
    let mut derivations = Vec::new();
    try_reserve_vec::<_, S::Endpoint>(
        &mut forward,
        left.input_endpoint_count(),
        RemapAllocation::ForwardHeaders,
    )?;
    try_reserve_vec::<_, S::Endpoint>(
        &mut reverse,
        right.output_endpoint_count(),
        RemapAllocation::ReverseHeaders,
    )?;

    let mut target_marks = HashMap::new();
    try_reserve_map::<S::Endpoint, usize, S::Endpoint>(
        &mut target_marks,
        right.output_endpoint_count(),
        RemapAllocation::CompositionDeduplication,
    )?;
    for (origin_ordinal, left_mapping) in left.outcomes().enumerate() {
        work.spend::<S::Endpoint>(1)?;
        let epoch = origin_ordinal
            .checked_add(1)
            .ok_or(RemapError::AccountingOverflow)?;
        let start = forward_targets.len();
        for intermediate in left_mapping.targets() {
            work.spend::<S::Endpoint>(1)?;
            let right_index = right_forward_index[intermediate];
            let right_mapping = right.origin_view(right_index);
            for target in right_mapping.targets() {
                work.spend::<S::Endpoint>(1)?;
                if mark_once(&mut target_marks, *target, epoch) {
                    push_bounded_cell(
                        &mut forward_targets,
                        *target,
                        limits.relation_cells,
                        RemapAllocation::ForwardTargets,
                    )?;
                }
            }
        }
        forward.push(ForwardHeader {
            endpoint: left_mapping.endpoint(),
            targets: start..forward_targets.len(),
        });
    }

    let origin_capacity = left.input_endpoint_count();
    let derivation_capacity = left
        .derivations
        .len()
        .checked_add(right.derivations.len())
        .ok_or(RemapError::AccountingOverflow)?;
    if derivation_capacity > limits.relation_cells {
        return Err(RemapError::LimitExceeded {
            limit: RemapLimit::RelationCells,
            maximum: limits.relation_cells,
        });
    }
    let mut origin_marks = HashMap::new();
    let mut derivation_marks = HashMap::new();
    try_reserve_map::<S::Endpoint, usize, S::Endpoint>(
        &mut origin_marks,
        origin_capacity,
        RemapAllocation::CompositionDeduplication,
    )?;
    try_reserve_map::<DerivationProvenance, usize, S::Endpoint>(
        &mut derivation_marks,
        derivation_capacity,
        RemapAllocation::CompositionDeduplication,
    )?;

    for (target_ordinal, right_mapping) in right.target_origins().enumerate() {
        work.spend::<S::Endpoint>(1)?;
        let epoch = target_ordinal
            .checked_add(1)
            .ok_or(RemapError::AccountingOverflow)?;
        let origin_start = reverse_origins.len();
        let derivation_start = derivations.len();
        for intermediate in right_mapping.origins() {
            work.spend::<S::Endpoint>(1)?;
            let left_index = left_reverse_index[intermediate];
            let left_mapping = left.target_view(left_index);
            for origin in left_mapping.origins() {
                work.spend::<S::Endpoint>(1)?;
                if mark_once(&mut origin_marks, *origin, epoch) {
                    push_bounded_cell(
                        &mut reverse_origins,
                        *origin,
                        limits.relation_cells,
                        RemapAllocation::ReverseOrigins,
                    )?;
                }
            }
            for derivation in left_mapping.derivations() {
                work.spend::<S::Endpoint>(1)?;
                if mark_once(&mut derivation_marks, *derivation, epoch) {
                    push_bounded_cell(
                        &mut derivations,
                        *derivation,
                        limits.relation_cells,
                        RemapAllocation::Derivations,
                    )?;
                }
            }
        }
        // Right-side originless contributions occur after inherited left-side paths.
        for derivation in right_mapping.derivations() {
            work.spend::<S::Endpoint>(1)?;
            if mark_once(&mut derivation_marks, *derivation, epoch) {
                push_bounded_cell(
                    &mut derivations,
                    *derivation,
                    limits.relation_cells,
                    RemapAllocation::Derivations,
                )?;
            }
        }
        reverse.push(ReverseHeader {
            endpoint: right_mapping.endpoint(),
            origins: origin_start..reverse_origins.len(),
            derivations: derivation_start..derivations.len(),
        });
    }

    Ok(ComposedRemap {
        envelope: RemapEnvelope {
            from_generation: left.from_generation,
            to_generation: right.to_generation,
            relation: RemapRelation::General {
                forward,
                forward_targets,
                reverse,
                reverse_origins,
            },
            derivations,
            work_units: work.used,
            derivation_rebind_work_units: 0,
            marker: PhantomData,
        },
    })
}

fn check_composition_envelope_limits<S: RemapSpace>(
    envelope: &RemapEnvelope<S>,
    limits: RemapLimits,
) -> Result<(), RemapError<S::Endpoint>> {
    if envelope.input_endpoint_count() > limits.endpoints_per_side
        || envelope.output_endpoint_count() > limits.endpoints_per_side
    {
        return Err(RemapError::LimitExceeded {
            limit: RemapLimit::EndpointsPerSide,
            maximum: limits.endpoints_per_side,
        });
    }
    if envelope.relation_cell_count() > limits.relation_cells
        || envelope.derivations.len() > limits.relation_cells
    {
        return Err(RemapError::LimitExceeded {
            limit: RemapLimit::RelationCells,
            maximum: limits.relation_cells,
        });
    }
    Ok(())
}

fn mark_once<K: Copy + Eq + Hash>(marks: &mut HashMap<K, usize>, key: K, epoch: usize) -> bool {
    match marks.get_mut(&key) {
        Some(last_epoch) if *last_epoch == epoch => false,
        Some(last_epoch) => {
            *last_epoch = epoch;
            true
        }
        None => {
            marks.insert(key, epoch);
            true
        }
    }
}

fn push_bounded_cell<T, E: fmt::Debug>(
    values: &mut Vec<T>,
    value: T,
    maximum: usize,
    resource: RemapAllocation,
) -> Result<(), RemapError<E>> {
    if values.len() == maximum {
        return Err(RemapError::LimitExceeded {
            limit: RemapLimit::RelationCells,
            maximum,
        });
    }
    ensure_push_capacity::<T, E>(values, resource)?;
    values.push(value);
    Ok(())
}

fn derivation_rebind_work<E: fmt::Debug>(count: usize) -> Result<usize, RemapError<E>> {
    count.checked_mul(2).ok_or(RemapError::AccountingOverflow)
}

fn checked_derivation_rebind_total<E: fmt::Debug>(
    current: usize,
    count: usize,
) -> Result<usize, RemapError<E>> {
    current
        .checked_add(derivation_rebind_work::<E>(count)?)
        .ok_or(RemapError::AccountingOverflow)
}

fn preflight_derivation_rebind<E: fmt::Debug>(
    derivations: &[DerivationProvenance],
    remap: &ProvenanceForkRemap,
    target_arena: &ProvenanceArena,
) -> Result<(), RemapError<E>> {
    for derivation in derivations {
        derivation
            .try_rebind(remap, target_arena)
            .map_err(|source| RemapError::DerivationRebind { source })?;
    }
    Ok(())
}

fn apply_derivation_rebind<E: fmt::Debug>(
    derivations: &mut [DerivationProvenance],
    remap: &ProvenanceForkRemap,
    target_arena: &ProvenanceArena,
) -> Result<(), RemapError<E>> {
    for derivation in derivations {
        *derivation = derivation
            .try_rebind(remap, target_arena)
            .map_err(|source| RemapError::DerivationRebind { source })?;
    }
    Ok(())
}

/// Ordered per-step remaps plus their deterministic input-to-final composition.
pub struct RemapSequence<S: RemapSpace> {
    steps: Vec<RemapStep<S>>,
    composed: Option<ComposedRemap<S>>,
    composition_work_units: usize,
    derivation_rebind_work_units: usize,
}

impl<S: RemapSpace> fmt::Debug for RemapSequence<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemapSequence")
            .field("subject", &S::SUBJECT)
            .field("steps", &self.steps.len())
            .field(
                "from_generation",
                &self.steps.first().map(|step| step.from_generation),
            )
            .field(
                "to_generation",
                &self.steps.last().map(|step| step.to_generation),
            )
            .field("composition_work_units", &self.composition_work_units)
            .field(
                "derivation_rebind_work_units",
                &self.derivation_rebind_work_units,
            )
            .finish()
    }
}

impl<S: RemapSpace> RemapSequence<S> {
    /// Builds an ordered sequence using the default hard limits.
    pub fn try_new<I>(steps: I) -> Result<Self, RemapError<S::Endpoint>>
    where
        I: IntoIterator<Item = RemapStep<S>>,
    {
        Self::try_new_with_limits(steps, RemapLimits::default())
    }

    /// Builds an ordered sequence and validates every boundary before composition.
    pub fn try_new_with_limits<I>(
        steps: I,
        limits: RemapLimits,
    ) -> Result<Self, RemapError<S::Endpoint>>
    where
        I: IntoIterator<Item = RemapStep<S>>,
    {
        let mut input = steps.into_iter();
        let (lower, upper) = input.size_hint();
        if lower > limits.steps {
            return Err(RemapError::LimitExceeded {
                limit: RemapLimit::Steps,
                maximum: limits.steps,
            });
        }
        let preflight = upper
            .filter(|count| *count <= limits.steps)
            .unwrap_or(lower);
        let mut work = WorkCounter::new(limits.work_units);
        let mut steps = Vec::new();
        try_reserve_vec::<_, S::Endpoint>(&mut steps, preflight, RemapAllocation::Steps)?;
        for step in input.by_ref() {
            if steps.len() == limits.steps {
                return Err(RemapError::LimitExceeded {
                    limit: RemapLimit::Steps,
                    maximum: limits.steps,
                });
            }
            ensure_push_capacity::<_, S::Endpoint>(&mut steps, RemapAllocation::Steps)?;
            work.spend::<S::Endpoint>(1)?;
            steps.push(step);
        }
        if steps.is_empty() {
            return Err(RemapError::EmptySequence);
        }

        for (index, pair) in steps.windows(2).enumerate() {
            work.spend::<S::Endpoint>(1)?;
            if pair[0].to_generation != pair[1].from_generation {
                return Err(RemapError::StepBoundaryMismatch {
                    step: index + 1,
                    expected: pair[0].to_generation,
                    actual: pair[1].from_generation,
                });
            }
        }

        let composed = if steps.len() == 1 {
            None
        } else {
            let mut composed = compose_pair(&steps[0].envelope, &steps[1], limits, &mut work)?;
            for step in &steps[2..] {
                composed = compose_pair(&composed.envelope, step, limits, &mut work)?;
            }
            Some(composed)
        };
        Ok(Self {
            steps,
            composed,
            composition_work_units: work.used,
            derivation_rebind_work_units: 0,
        })
    }

    pub fn steps(&self) -> impl ExactSizeIterator<Item = &RemapStep<S>> {
        self.steps.iter()
    }

    /// Returns the input-to-final relation. A one-step sequence reuses its sole envelope.
    pub fn composed(&self) -> &RemapEnvelope<S> {
        self.composed
            .as_ref()
            .map(|composed| &composed.envelope)
            .unwrap_or(&self.steps[0].envelope)
    }

    pub const fn composition_work_units(&self) -> usize {
        self.composition_work_units
    }

    /// Returns cumulative work spent rebinding every stored attachment copy.
    pub const fn derivation_rebind_work_units(&self) -> usize {
        self.derivation_rebind_work_units
    }

    /// Measures the sequence vector and every initialized envelope buffer.
    ///
    /// The cached composed envelope is counted because it owns independent
    /// relation buffers. Spare `steps` capacity is counted by slot size but is
    /// never treated as initialized nested storage.
    pub fn retained_footprint(&self) -> Result<RemapRetainedFootprint, RemapError<S::Endpoint>> {
        let mut footprint = RemapRetainedFootprint::default();
        footprint.try_add::<RemapStep<S>, S::Endpoint>(self.steps.capacity())?;
        for step in &self.steps {
            footprint = footprint.try_merge::<S::Endpoint>(step.retained_footprint()?)?;
        }
        if let Some(composed) = &self.composed {
            footprint =
                footprint.try_merge::<S::Endpoint>(composed.envelope.retained_footprint()?)?;
        }
        Ok(footprint)
    }

    /// Returns allocator-retained typed slots, including unused capacity.
    pub fn retained_allocation_cells(&self) -> Result<usize, RemapError<S::Endpoint>> {
        Ok(self.retained_footprint()?.cells())
    }

    /// Returns checked allocator-retained bytes, including unused capacity.
    pub fn retained_allocation_bytes(&self) -> Result<usize, RemapError<S::Endpoint>> {
        Ok(self.retained_footprint()?.bytes())
    }

    /// Rebinds all step and cached-composition derivations using hard defaults.
    pub fn try_rebind_derivations(
        self,
        remap: &ProvenanceForkRemap,
        target_arena: &ProvenanceArena,
    ) -> Result<Self, RemapError<S::Endpoint>> {
        self.try_rebind_derivations_with_limits(remap, target_arena, RemapLimits::default())
    }

    /// Rebinds all retained attachment copies under explicit limits.
    ///
    /// Every source attachment and target arena resolution is prevalidated
    /// before the first stored value changes. The sequence is consumed so even
    /// an impossible second-pass remap error cannot expose partial state.
    pub fn try_rebind_derivations_with_limits(
        mut self,
        remap: &ProvenanceForkRemap,
        target_arena: &ProvenanceArena,
        limits: RemapLimits,
    ) -> Result<Self, RemapError<S::Endpoint>> {
        if self.steps.len() > limits.steps {
            return Err(RemapError::LimitExceeded {
                limit: RemapLimit::Steps,
                maximum: limits.steps,
            });
        }

        let mut derivation_count = 0usize;
        for step in &self.steps {
            check_composition_envelope_limits(&step.envelope, limits)?;
            derivation_count = derivation_count
                .checked_add(step.envelope.derivations.len())
                .ok_or(RemapError::AccountingOverflow)?;
        }
        if let Some(composed) = &self.composed {
            check_composition_envelope_limits(&composed.envelope, limits)?;
            derivation_count = derivation_count
                .checked_add(composed.envelope.derivations.len())
                .ok_or(RemapError::AccountingOverflow)?;
        }

        let required_work = derivation_rebind_work::<S::Endpoint>(derivation_count)?;
        let mut work = WorkCounter::new(limits.work_units);
        work.spend::<S::Endpoint>(required_work)?;
        let cumulative_work = self
            .derivation_rebind_work_units
            .checked_add(required_work)
            .ok_or(RemapError::AccountingOverflow)?;
        for step in &self.steps {
            let _projected_work = checked_derivation_rebind_total::<S::Endpoint>(
                step.envelope.derivation_rebind_work_units,
                step.envelope.derivations.len(),
            )?;
            preflight_derivation_rebind::<S::Endpoint>(
                &step.envelope.derivations,
                remap,
                target_arena,
            )?;
        }
        if let Some(composed) = &self.composed {
            let _projected_work = checked_derivation_rebind_total::<S::Endpoint>(
                composed.envelope.derivation_rebind_work_units,
                composed.envelope.derivations.len(),
            )?;
            preflight_derivation_rebind::<S::Endpoint>(
                &composed.envelope.derivations,
                remap,
                target_arena,
            )?;
        }

        for step in &mut self.steps {
            apply_derivation_rebind::<S::Endpoint>(
                &mut step.envelope.derivations,
                remap,
                target_arena,
            )?;
            step.envelope.derivation_rebind_work_units =
                checked_derivation_rebind_total::<S::Endpoint>(
                    step.envelope.derivation_rebind_work_units,
                    step.envelope.derivations.len(),
                )?;
        }
        if let Some(composed) = &mut self.composed {
            apply_derivation_rebind::<S::Endpoint>(
                &mut composed.envelope.derivations,
                remap,
                target_arena,
            )?;
            composed.envelope.derivation_rebind_work_units =
                checked_derivation_rebind_total::<S::Endpoint>(
                    composed.envelope.derivation_rebind_work_units,
                    composed.envelope.derivations.len(),
                )?;
        }
        self.derivation_rebind_work_units = cumulative_work;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::{
        OperationKind, OptionsDigest, ProvenanceArenaBuilder, test_provenance_fixture,
    };

    fn generation() -> SceneGenerationId {
        SceneGenerationId::allocate().expect("generation space should remain available")
    }

    fn primitive(generation: SceneGenerationId, owner: u32, local: u32) -> PrimitiveId {
        PrimitiveId::new(LogicalMeshId::new(generation, owner), local)
    }

    fn vertex(generation: SceneGenerationId, index: u32) -> VertexEndpoint {
        VertexEndpoint::new(primitive(generation, 0, 0), index)
    }

    fn vertex_with_owner(generation: SceneGenerationId, owner: u32, index: u32) -> VertexEndpoint {
        VertexEndpoint::new(primitive(generation, owner, 0), index)
    }

    fn generated_derivation(byte: u8) -> (ProvenanceArena, DerivationProvenance) {
        let mut builder = ProvenanceArenaBuilder::try_new()
            .expect("provenance arena identity should be available");
        let attachment = builder
            .add_operation(
                OperationIdentity::without_evidence(
                    OperationKind::Generated,
                    OptionsDigest::sha256([byte; 32]),
                ),
                std::iter::empty::<Provenance>(),
            )
            .expect("originless generated operation should be valid");
        let arena = builder.finish().expect("generated arena should validate");
        let derivation = DerivationProvenance::try_new(&arena, attachment)
            .expect("operation attachment should be accepted");
        (arena, derivation)
    }

    #[test]
    fn every_entity_and_domain_uses_a_typed_remap_space() {
        assert_eq!(
            NodeRemapSpace::SUBJECT,
            RemapSubject::Entity(SceneEntityKind::Node)
        );
        assert_eq!(
            MeshInstanceRemapSpace::SUBJECT,
            RemapSubject::Entity(SceneEntityKind::MeshInstance)
        );
        assert_eq!(
            LogicalMeshRemapSpace::SUBJECT,
            RemapSubject::Entity(SceneEntityKind::LogicalMesh)
        );
        assert_eq!(
            PrimitiveRemapSpace::SUBJECT,
            RemapSubject::Entity(SceneEntityKind::Primitive)
        );
        assert_eq!(
            MorphTargetRemapSpace::SUBJECT,
            RemapSubject::Entity(SceneEntityKind::MorphTarget)
        );
        assert_eq!(
            MaterialRemapSpace::SUBJECT,
            RemapSubject::Entity(SceneEntityKind::Material)
        );
        assert_eq!(
            ImageRemapSpace::SUBJECT,
            RemapSubject::Entity(SceneEntityKind::Image)
        );
        assert_eq!(
            SamplerRemapSpace::SUBJECT,
            RemapSubject::Entity(SceneEntityKind::Sampler)
        );
        assert_eq!(
            TextureRemapSpace::SUBJECT,
            RemapSubject::Entity(SceneEntityKind::Texture)
        );
        assert_eq!(
            SkinRemapSpace::SUBJECT,
            RemapSubject::Entity(SceneEntityKind::Skin)
        );
        assert_eq!(
            AnimationClipRemapSpace::SUBJECT,
            RemapSubject::Entity(SceneEntityKind::AnimationClip)
        );
        assert_eq!(
            CameraRemapSpace::SUBJECT,
            RemapSubject::Entity(SceneEntityKind::Camera)
        );
        assert_eq!(
            LightRemapSpace::SUBJECT,
            RemapSubject::Entity(SceneEntityKind::Light)
        );
        assert_eq!(
            VertexRemapSpace::SUBJECT,
            RemapSubject::Domain(AttributeDomain::Vertex)
        );
        assert_eq!(
            CornerRemapSpace::SUBJECT,
            RemapSubject::Domain(AttributeDomain::Corner)
        );
        assert_eq!(
            FaceRemapSpace::SUBJECT,
            RemapSubject::Domain(AttributeDomain::Face)
        );
        assert_eq!(
            InstanceRemapSpace::SUBJECT,
            RemapSubject::Domain(AttributeDomain::Instance)
        );

        let generation = generation();
        let primitive = primitive(generation, 7, 3);
        let vertex = VertexEndpoint::new(primitive, 11);
        let corner = CornerEndpoint::new(primitive, 13);
        let face = FaceEndpoint::new(primitive, 17);
        assert_eq!(vertex.primitive(), primitive);
        assert_eq!(vertex.domain_index(), 11);
        assert_eq!(corner.primitive(), primitive);
        assert_eq!(corner.domain_index(), 13);
        assert_eq!(face.primitive(), primitive);
        assert_eq!(face.domain_index(), 17);
    }

    #[test]
    fn changed_builder_canonicalizes_outer_order_and_preserves_relation_order() {
        let from_generation = generation();
        let to_generation = generation();
        let old = (0..5)
            .map(|index| vertex(from_generation, index))
            .collect::<Vec<_>>();
        let new = (0..5)
            .map(|index| vertex(to_generation, index))
            .collect::<Vec<_>>();
        let (_arena, derivation) = generated_derivation(7);
        let mut builder = RemapBuilder::<VertexRemapSpace>::try_new(
            from_generation,
            to_generation,
            [old[4], old[1], old[3], old[0], old[2]],
            [new[3], new[1], new[4], new[0], new[2]],
        )
        .expect("obligations should canonicalize");

        builder.try_delete(old[0]).unwrap();
        builder.try_split(old[1], [new[2], new[1]]).unwrap();
        builder.try_merge([old[3], old[2]], new[0]).unwrap();
        builder.try_map(old[4], new[3]).unwrap();
        builder.try_generate(new[4], derivation).unwrap();
        let remap = builder.try_finish().expect("all obligations are complete");

        assert_eq!(remap.change(), RemapChange::Changed);
        assert_eq!(remap.input_endpoint_count(), 5);
        assert_eq!(remap.output_endpoint_count(), 5);
        assert_eq!(remap.relation_cell_count(), 5);
        assert_eq!(
            remap
                .outcomes()
                .map(|view| view.endpoint())
                .collect::<Vec<_>>(),
            old
        );
        assert!(remap.outcome(old[0]).unwrap().is_deleted());
        assert_eq!(remap.outcome(old[1]).unwrap().targets(), &[new[2], new[1]]);
        assert_eq!(remap.origins(new[0]).unwrap().origins(), &[old[3], old[2]]);
        let generated = remap.origins(new[4]).unwrap();
        assert!(generated.is_originless());
        assert_eq!(generated.origins(), &[]);
        assert_eq!(generated.derivations(), &[derivation]);
    }

    #[test]
    fn domain_obligations_canonicalize_by_complete_owner_chain() {
        let from_generation = generation();
        let to_generation = generation();
        let canonical_origins = [
            VertexEndpoint::new(primitive(from_generation, 0, 0), 3),
            VertexEndpoint::new(primitive(from_generation, 0, 1), 2),
            VertexEndpoint::new(primitive(from_generation, 1, 0), 1),
            VertexEndpoint::new(primitive(from_generation, 1, 0), 5),
        ];
        let canonical_targets = canonical_origins.map(|origin| {
            VertexEndpoint::new(
                origin.primitive().rebind(to_generation),
                origin.domain_index(),
            )
        });
        let mut builder = RemapBuilder::<VertexRemapSpace>::try_new(
            from_generation,
            to_generation,
            [
                canonical_origins[3],
                canonical_origins[1],
                canonical_origins[2],
                canonical_origins[0],
            ],
            [
                canonical_targets[2],
                canonical_targets[0],
                canonical_targets[3],
                canonical_targets[1],
            ],
        )
        .unwrap();
        for (origin, target) in canonical_origins.into_iter().zip(canonical_targets) {
            builder.try_map(origin, target).unwrap();
        }

        let remap = builder.try_finish().unwrap();
        assert_eq!(
            remap
                .outcomes()
                .map(|mapping| mapping.endpoint())
                .collect::<Vec<_>>(),
            canonical_origins
        );
        assert_eq!(
            remap
                .target_origins()
                .map(|mapping| mapping.endpoint())
                .collect::<Vec<_>>(),
            canonical_targets
        );
    }

    fn one_to_one_construction_work(endpoint_count: u32) -> usize {
        let from_generation = generation();
        let to_generation = generation();
        let old = (0..endpoint_count)
            .map(|index| vertex(from_generation, index))
            .collect::<Vec<_>>();
        let new = (0..endpoint_count)
            .map(|index| vertex(to_generation, index))
            .collect::<Vec<_>>();
        let mut builder = RemapBuilder::<VertexRemapSpace>::try_new(
            from_generation,
            to_generation,
            old.iter().rev().copied(),
            new.iter().rev().copied(),
        )
        .expect("reverse-ordered obligations should canonicalize");
        for (origin, target) in old.into_iter().zip(new) {
            builder
                .try_map(origin, target)
                .expect("one-to-one obligations should map exactly once");
        }
        builder
            .try_finish()
            .expect("complete one-to-one remap should finish")
            .work_units()
    }

    #[test]
    fn direct_construction_work_scales_linearly_with_obligations() {
        const SMALL_ENDPOINTS: u32 = 512;
        const LARGE_ENDPOINTS: u32 = 4_096;
        const SCALE: usize = (LARGE_ENDPOINTS / SMALL_ENDPOINTS) as usize;

        let small_work = one_to_one_construction_work(SMALL_ENDPOINTS);
        let large_work = one_to_one_construction_work(LARGE_ENDPOINTS);

        assert!(
            large_work <= small_work * SCALE,
            "8x obligations must use at most 8x deterministic construction work: small={small_work}, large={large_work}"
        );
    }

    #[test]
    fn builder_rejects_duplicate_conflicting_and_omitted_obligations() {
        let from_generation = generation();
        let to_generation = generation();
        let old = vertex(from_generation, 0);
        let new = vertex(to_generation, 0);
        assert_eq!(
            RemapBuilder::<VertexRemapSpace>::try_new(
                from_generation,
                to_generation,
                [old, old],
                [new],
            )
            .unwrap_err(),
            RemapError::DuplicateObligation {
                side: RemapSide::From,
                endpoint: old,
            }
        );

        let mut conflicting =
            RemapBuilder::<VertexRemapSpace>::try_new(from_generation, to_generation, [old], [new])
                .unwrap();
        conflicting.try_map(old, new).unwrap();
        assert_eq!(
            conflicting.try_delete(old),
            Err(RemapError::ConflictingDisposition {
                side: RemapSide::From,
                endpoint: old,
            })
        );

        let omitted =
            RemapBuilder::<VertexRemapSpace>::try_new(from_generation, to_generation, [old], [new])
                .unwrap();
        assert_eq!(
            omitted.try_finish().unwrap_err(),
            RemapError::OmittedObligation {
                side: RemapSide::From,
                endpoint: old,
            }
        );
    }

    #[test]
    fn duplicate_clause_endpoints_are_not_silently_deduplicated() {
        let from_generation = generation();
        let to_generation = generation();
        let old = vertex(from_generation, 0);
        let new_a = vertex(to_generation, 0);
        let new_b = vertex(to_generation, 1);
        let mut builder = RemapBuilder::<VertexRemapSpace>::try_new(
            from_generation,
            to_generation,
            [old],
            [new_a, new_b],
        )
        .unwrap();
        assert_eq!(
            builder.try_split(old, [new_a, new_a]),
            Err(RemapError::DuplicateClauseEndpoint {
                side: RemapSide::To,
                endpoint: new_a,
            })
        );
        assert_eq!(
            builder.try_split(old, [new_a, new_b]),
            Ok(()),
            "a rejected clause must not partially consume obligations"
        );
    }

    #[test]
    fn generation_mismatch_precedes_endpoint_membership_lookup() {
        let from_generation = generation();
        let output_generation = generation();
        let sibling_generation = generation();
        let old = vertex(from_generation, 0);
        let new = vertex(output_generation, 0);
        let sibling = vertex(sibling_generation, u32::MAX);
        let mut builder = RemapBuilder::<VertexRemapSpace>::try_new(
            from_generation,
            output_generation,
            [old],
            [new],
        )
        .unwrap();
        assert_eq!(
            builder.try_map(old, sibling),
            Err(RemapError::EndpointGenerationMismatch {
                side: RemapSide::To,
                expected: output_generation,
                actual: sibling_generation,
            })
        );
        builder.try_map(old, new).unwrap();
        let remap = builder.try_finish().unwrap();
        assert_eq!(
            remap.origins(sibling).unwrap_err(),
            RemapError::EndpointGenerationMismatch {
                side: RemapSide::To,
                expected: output_generation,
                actual: sibling_generation,
            }
        );
    }

    #[test]
    fn complete_owner_chain_participates_in_endpoint_membership() {
        let from_generation = generation();
        let to_generation = generation();
        let expected = vertex_with_owner(from_generation, 0, 0);
        let foreign_owner = vertex_with_owner(from_generation, 1, 0);
        let target = vertex_with_owner(to_generation, 0, 0);
        let mut builder = RemapBuilder::<VertexRemapSpace>::try_new(
            from_generation,
            to_generation,
            [expected],
            [target],
        )
        .unwrap();
        assert_eq!(
            builder.try_map(foreign_owner, target),
            Err(RemapError::UnknownObligation {
                side: RemapSide::From,
                endpoint: foreign_owner,
            })
        );
    }

    #[test]
    fn envelope_rejects_same_generation_boundaries() {
        let generation = generation();
        assert_eq!(
            RemapBuilder::<VertexRemapSpace>::try_new(
                generation,
                generation,
                std::iter::empty(),
                std::iter::empty(),
            )
            .unwrap_err(),
            RemapError::SameGeneration
        );
    }

    #[test]
    fn explicit_unchanged_is_an_identity_relation_across_generations() {
        let from_generation = generation();
        let to_generation = generation();
        let old = [vertex(from_generation, 0), vertex(from_generation, 1)];
        let new = [vertex(to_generation, 0), vertex(to_generation, 1)];
        let remap = RemapBuilder::<VertexRemapSpace>::try_new(
            from_generation,
            to_generation,
            [old[1], old[0]],
            [new[1], new[0]],
        )
        .unwrap()
        .try_finish_explicitly_unchanged()
        .unwrap();

        assert!(remap.is_explicitly_unchanged());
        assert_eq!(remap.outcome(old[0]).unwrap().targets(), &[new[0]]);
        assert_eq!(remap.origins(new[1]).unwrap().origins(), &[old[1]]);

        let mismatch = RemapBuilder::<VertexRemapSpace>::try_new(
            from_generation,
            to_generation,
            old,
            [new[0]],
        )
        .unwrap();
        assert_eq!(
            mismatch.try_finish_explicitly_unchanged().unwrap_err(),
            RemapError::ExplicitUnchangedCardinality
        );
    }

    #[test]
    fn compact_unchanged_marker_rebinds_the_complete_owner_chain() {
        let from_generation = generation();
        let to_generation = generation();
        let endpoint = VertexEndpoint::new(primitive(from_generation, 7, 3), 11);
        let boundary = RemapGenerationBoundary::try_new(from_generation, to_generation)
            .expect("distinct generations should admit an unchanged boundary");
        let marker = ExplicitUnchangedRemap::<VertexRemapSpace>::from_boundary(boundary);

        let rebound = marker
            .try_rebind(endpoint)
            .expect("input endpoint should rebind");
        assert_eq!(marker.input_generation(), from_generation);
        assert_eq!(marker.output_generation(), to_generation);
        assert_eq!(marker.change(), RemapChange::ExplicitlyUnchanged);
        assert_eq!(rebound.primitive().generation(), to_generation);
        assert_eq!(rebound.primitive().logical_mesh().index(), 7);
        assert_eq!(rebound.primitive().local(), 3);
        assert_eq!(rebound.domain_index(), 11);

        let foreign = VertexEndpoint::new(primitive(generation(), 7, 3), 11);
        assert!(matches!(
            marker.try_rebind(foreign),
            Err(RemapError::EndpointGenerationMismatch {
                side: RemapSide::From,
                expected,
                ..
            }) if expected == from_generation
        ));
    }

    #[test]
    fn derivation_provenance_rejects_source_and_foreign_attachments() {
        let (source_arena, source) = test_provenance_fixture();
        assert_eq!(
            DerivationProvenance::try_new(&source_arena, source),
            Err(DerivationProvenanceError::SourceAttachment)
        );
        let (operation_arena, derivation) = generated_derivation(9);
        assert_eq!(
            DerivationProvenance::try_new(&source_arena, derivation.attachment()),
            Err(DerivationProvenanceError::Access(
                ProvenanceAccessError::ArenaMismatch
            ))
        );
        assert_eq!(
            derivation
                .operation_identity(&operation_arena)
                .unwrap()
                .kind(),
            &OperationKind::Generated
        );
    }

    #[test]
    fn derivation_rebind_preserves_identity_and_requires_the_matching_fork_arena() {
        let (source_arena, derivation) = generated_derivation(31);
        let fork = source_arena.try_fork().unwrap();
        let remap = fork.remap().clone();
        let (target_arena, finished_remap) = fork.finish().unwrap();
        assert_eq!(remap, finished_remap);

        let rebound = derivation.try_rebind(&remap, &target_arena).unwrap();
        assert_eq!(rebound, derivation);
        assert!(rebound.operation_identity(&target_arena).is_ok());
        assert!(rebound.operation_identity(&source_arena).is_ok());
        assert_eq!(
            derivation.try_rebind(&remap, &source_arena).unwrap_err(),
            DerivationProvenanceRebindError::Remap(ProvenanceRemapError::ForeignTargetArena)
        );
    }

    #[test]
    fn composition_preserves_split_merge_order_and_generated_provenance() {
        let generation_0 = generation();
        let generation_1 = generation();
        let generation_2 = generation();
        let old = (0..5)
            .map(|index| vertex(generation_0, index))
            .collect::<Vec<_>>();
        let middle = (0..5)
            .map(|index| vertex(generation_1, index))
            .collect::<Vec<_>>();
        let final_endpoints = (0..5)
            .map(|index| vertex(generation_2, index))
            .collect::<Vec<_>>();
        let (_first_arena, first_derivation) = generated_derivation(11);
        let (_second_arena, second_derivation) = generated_derivation(12);

        let mut first = RemapBuilder::<VertexRemapSpace>::try_new(
            generation_0,
            generation_1,
            old.iter().copied(),
            middle.iter().copied(),
        )
        .unwrap();
        first.try_map(old[0], middle[0]).unwrap();
        first.try_map(old[1], middle[1]).unwrap();
        first.try_map(old[2], middle[2]).unwrap();
        first.try_delete(old[3]).unwrap();
        first.try_map(old[4], middle[3]).unwrap();
        first.try_generate(middle[4], first_derivation).unwrap();
        let first = first.try_finish().unwrap();

        let mut second = RemapBuilder::<VertexRemapSpace>::try_new(
            generation_1,
            generation_2,
            middle.iter().copied(),
            final_endpoints.iter().copied(),
        )
        .unwrap();
        second
            .try_merge([middle[1], middle[0]], final_endpoints[0])
            .unwrap();
        second
            .try_split(middle[2], [final_endpoints[2], final_endpoints[1]])
            .unwrap();
        second
            .try_merge([middle[4], middle[3]], final_endpoints[3])
            .unwrap();
        second
            .try_generate(final_endpoints[4], second_derivation)
            .unwrap();
        let second = second.try_finish().unwrap();

        let sequence = RemapSequence::try_new([first, second]).unwrap();
        let composed = sequence.composed();
        assert_eq!(sequence.steps().len(), 2);
        assert_eq!(composed.input_generation(), generation_0);
        assert_eq!(composed.output_generation(), generation_2);
        assert_eq!(
            composed.origins(final_endpoints[0]).unwrap().origins(),
            &[old[1], old[0]],
            "merge origin order must follow operation order"
        );
        assert_eq!(
            composed.outcome(old[2]).unwrap().targets(),
            &[final_endpoints[2], final_endpoints[1]],
            "split target order must follow operation order"
        );
        assert!(composed.outcome(old[3]).unwrap().is_deleted());
        let mixed = composed.origins(final_endpoints[3]).unwrap();
        assert_eq!(mixed.origins(), &[old[4]]);
        assert_eq!(mixed.derivations(), &[first_derivation]);
        let generated = composed.origins(final_endpoints[4]).unwrap();
        assert!(generated.is_originless());
        assert_eq!(generated.derivations(), &[second_derivation]);
        assert!(sequence.composition_work_units() > 0);
    }

    #[test]
    fn nonempty_unchanged_then_changed_composition_preserves_right_hand_semantics() {
        let generation_0 = generation();
        let generation_1 = generation();
        let generation_2 = generation();
        let old = [vertex(generation_0, 0), vertex(generation_0, 1)];
        let middle = [vertex(generation_1, 0), vertex(generation_1, 1)];
        let final_endpoints = [
            vertex(generation_2, 0),
            vertex(generation_2, 1),
            vertex(generation_2, 2),
            vertex(generation_2, 3),
        ];
        let (_arena, generated) = generated_derivation(71);
        let unchanged =
            RemapBuilder::<VertexRemapSpace>::try_new(generation_0, generation_1, old, middle)
                .unwrap()
                .try_finish_explicitly_unchanged()
                .unwrap();
        let mut changed = RemapBuilder::<VertexRemapSpace>::try_new(
            generation_1,
            generation_2,
            middle,
            final_endpoints,
        )
        .unwrap();
        changed
            .try_split(middle[0], [final_endpoints[1], final_endpoints[0]])
            .unwrap();
        changed.try_map(middle[1], final_endpoints[2]).unwrap();
        changed.try_generate(final_endpoints[3], generated).unwrap();

        let sequence = RemapSequence::try_new([unchanged, changed.try_finish().unwrap()]).unwrap();
        let composed = sequence.composed();
        assert_eq!(composed.input_generation(), generation_0);
        assert_eq!(composed.output_generation(), generation_2);
        assert_eq!(composed.change(), RemapChange::Changed);
        assert_eq!(
            composed.outcome(old[0]).unwrap().targets(),
            &[final_endpoints[1], final_endpoints[0]]
        );
        assert_eq!(
            composed.origins(final_endpoints[1]).unwrap().origins(),
            &[old[0]]
        );
        let generated_output = composed.origins(final_endpoints[3]).unwrap();
        assert!(generated_output.is_originless());
        assert_eq!(generated_output.derivations(), &[generated]);
    }

    #[test]
    fn nonempty_changed_then_unchanged_composition_carries_left_hand_derivation() {
        let generation_0 = generation();
        let generation_1 = generation();
        let generation_2 = generation();
        let old = [vertex(generation_0, 0), vertex(generation_0, 1)];
        let middle = [vertex(generation_1, 0), vertex(generation_1, 1)];
        let final_endpoints = [vertex(generation_2, 0), vertex(generation_2, 1)];
        let (_arena, generated) = generated_derivation(72);
        let mut changed =
            RemapBuilder::<VertexRemapSpace>::try_new(generation_0, generation_1, old, middle)
                .unwrap();
        changed.try_merge([old[1], old[0]], middle[0]).unwrap();
        changed.try_generate(middle[1], generated).unwrap();
        let unchanged = RemapBuilder::<VertexRemapSpace>::try_new(
            generation_1,
            generation_2,
            middle,
            final_endpoints,
        )
        .unwrap()
        .try_finish_explicitly_unchanged()
        .unwrap();

        let sequence = RemapSequence::try_new([changed.try_finish().unwrap(), unchanged]).unwrap();
        let composed = sequence.composed();
        assert_eq!(composed.input_generation(), generation_0);
        assert_eq!(composed.output_generation(), generation_2);
        assert_eq!(composed.change(), RemapChange::Changed);
        assert_eq!(
            composed.outcome(old[1]).unwrap().targets(),
            &[final_endpoints[0]]
        );
        assert_eq!(
            composed.origins(final_endpoints[0]).unwrap().origins(),
            &[old[1], old[0]]
        );
        let generated_output = composed.origins(final_endpoints[1]).unwrap();
        assert!(generated_output.is_originless());
        assert_eq!(generated_output.derivations(), &[generated]);
    }

    fn merge_split_chain(size: u32) -> RemapSequence<VertexRemapSpace> {
        let generation_0 = generation();
        let generation_1 = generation();
        let generation_2 = generation();
        let generation_3 = generation();
        let generation_4 = generation();
        let origins = (0..size)
            .map(|index| vertex(generation_0, index))
            .collect::<Vec<_>>();
        let merged = vertex(generation_1, 0);
        let middle = (0..size)
            .map(|index| vertex(generation_2, index))
            .collect::<Vec<_>>();
        let merged_again = vertex(generation_3, 0);
        let targets = (0..size)
            .map(|index| vertex(generation_4, index))
            .collect::<Vec<_>>();

        let mut merge = RemapBuilder::<VertexRemapSpace>::try_new(
            generation_0,
            generation_1,
            origins.iter().copied(),
            [merged],
        )
        .unwrap();
        merge.try_merge(origins, merged).unwrap();
        let merge: RemapStep<VertexRemapSpace> = merge.try_finish().unwrap();

        let mut split = RemapBuilder::<VertexRemapSpace>::try_new(
            generation_1,
            generation_2,
            [merged],
            middle.iter().copied(),
        )
        .unwrap();
        split.try_split(merged, middle.iter().copied()).unwrap();
        let split: RemapStep<VertexRemapSpace> = split.try_finish().unwrap();

        let mut merge_again = RemapBuilder::<VertexRemapSpace>::try_new(
            generation_2,
            generation_3,
            middle.iter().copied(),
            [merged_again],
        )
        .unwrap();
        merge_again.try_merge(middle, merged_again).unwrap();
        let merge_again: RemapStep<VertexRemapSpace> = merge_again.try_finish().unwrap();

        let mut split_again = RemapBuilder::<VertexRemapSpace>::try_new(
            generation_3,
            generation_4,
            [merged_again],
            targets.iter().copied(),
        )
        .unwrap();
        split_again.try_split(merged_again, targets).unwrap();
        let split_again: RemapStep<VertexRemapSpace> = split_again.try_finish().unwrap();

        RemapSequence::try_new([merge, split, merge_again, split_again]).unwrap()
    }

    #[test]
    fn direct_step_left_fold_scales_with_materialized_relation_cells() {
        let small = merge_split_chain(16);
        let large = merge_split_chain(32);

        assert_eq!(small.composed().relation_cell_count(), 16 * 16);
        assert_eq!(large.composed().relation_cell_count(), 32 * 32);
        assert!(
            large.composition_work_units() <= small.composition_work_units() * 4,
            "2x dense merge/split width must use at most 4x composition work"
        );
    }

    #[test]
    fn sequence_rebinds_step_and_composed_derivations_to_one_fresh_arena() {
        let mut provenance_builder = ProvenanceArenaBuilder::try_new().unwrap();
        let first_attachment = provenance_builder
            .add_operation(
                OperationIdentity::without_evidence(
                    OperationKind::Generated,
                    OptionsDigest::sha256([41; 32]),
                ),
                std::iter::empty::<Provenance>(),
            )
            .unwrap();
        let second_attachment = provenance_builder
            .add_operation(
                OperationIdentity::without_evidence(
                    OperationKind::Generated,
                    OptionsDigest::sha256([42; 32]),
                ),
                [first_attachment],
            )
            .unwrap();
        let source_arena = provenance_builder.finish().unwrap();
        let first_derivation =
            DerivationProvenance::try_new(&source_arena, first_attachment).unwrap();
        let second_derivation =
            DerivationProvenance::try_new(&source_arena, second_attachment).unwrap();

        let generation_0 = generation();
        let generation_1 = generation();
        let generation_2 = generation();
        let old = vertex(generation_0, 0);
        let middle = [vertex(generation_1, 0), vertex(generation_1, 1)];
        let final_endpoints = [
            vertex(generation_2, 0),
            vertex(generation_2, 1),
            vertex(generation_2, 2),
        ];

        let mut first =
            RemapBuilder::<VertexRemapSpace>::try_new(generation_0, generation_1, [old], middle)
                .unwrap();
        first.try_map(old, middle[0]).unwrap();
        first.try_generate(middle[1], first_derivation).unwrap();

        let mut second = RemapBuilder::<VertexRemapSpace>::try_new(
            generation_1,
            generation_2,
            middle,
            final_endpoints,
        )
        .unwrap();
        second.try_map(middle[0], final_endpoints[0]).unwrap();
        second.try_map(middle[1], final_endpoints[1]).unwrap();
        second
            .try_generate(final_endpoints[2], second_derivation)
            .unwrap();
        let sequence =
            RemapSequence::try_new([first.try_finish().unwrap(), second.try_finish().unwrap()])
                .unwrap();

        let step_footprint = sequence
            .steps()
            .try_fold(RemapRetainedFootprint::default(), |total, step| {
                total.try_merge::<VertexEndpoint>(step.retained_footprint().unwrap())
            })
            .unwrap();
        let composed_footprint = sequence.composed().retained_footprint().unwrap();
        let sequence_footprint = sequence.retained_footprint().unwrap();
        assert_eq!(
            sequence_footprint.cells(),
            sequence.steps.capacity() + step_footprint.cells() + composed_footprint.cells()
        );
        assert_eq!(
            sequence_footprint.bytes(),
            sequence.steps.capacity() * std::mem::size_of::<RemapStep<VertexRemapSpace>>()
                + step_footprint.bytes()
                + composed_footprint.bytes()
        );

        let fork = source_arena.try_fork().unwrap();
        let remap = fork.remap().clone();
        let (target_arena, _) = fork.finish().unwrap();
        let rebound = sequence
            .try_rebind_derivations(&remap, &target_arena)
            .unwrap();

        assert_eq!(rebound.derivation_rebind_work_units(), 8);
        assert_eq!(rebound.steps().len(), 2);
        assert_eq!(
            rebound
                .steps()
                .map(|step| step.derivation_rebind_work_units())
                .collect::<Vec<_>>(),
            [2, 2]
        );
        assert_eq!(rebound.composed().derivation_rebind_work_units(), 4);
        assert_eq!(
            rebound
                .steps()
                .next()
                .unwrap()
                .origins(middle[1])
                .unwrap()
                .derivations()[0]
                .operation_identity(&target_arena)
                .unwrap()
                .options_digest(),
            &OptionsDigest::sha256([41; 32])
        );
        assert_eq!(
            rebound
                .steps()
                .nth(1)
                .unwrap()
                .origins(final_endpoints[2])
                .unwrap()
                .derivations()[0]
                .operation_identity(&target_arena)
                .unwrap()
                .options_digest(),
            &OptionsDigest::sha256([42; 32])
        );
        let inherited = rebound
            .composed()
            .origins(final_endpoints[1])
            .unwrap()
            .derivations()[0];
        let generated = rebound
            .composed()
            .origins(final_endpoints[2])
            .unwrap()
            .derivations()[0];
        assert_eq!(inherited, first_derivation);
        assert_eq!(generated, second_derivation);
        assert!(inherited.operation_identity(&target_arena).is_ok());
        assert!(generated.operation_identity(&target_arena).is_ok());
        assert!(inherited.operation_identity(&source_arena).is_ok());
        assert!(generated.operation_identity(&source_arena).is_ok());
    }

    #[test]
    fn derivation_rebind_obeys_work_limits_before_mutation() {
        let (source_arena, derivation) = generated_derivation(51);
        let from_generation = generation();
        let to_generation = generation();
        let target = vertex(to_generation, 0);
        let mut builder = RemapBuilder::<VertexRemapSpace>::try_new(
            from_generation,
            to_generation,
            std::iter::empty(),
            [target],
        )
        .unwrap();
        builder.try_generate(target, derivation).unwrap();
        let sequence = RemapSequence::try_new([builder.try_finish().unwrap()]).unwrap();
        let fork = source_arena.try_fork().unwrap();
        let remap = fork.remap().clone();
        let (target_arena, _) = fork.finish().unwrap();
        let limits = RemapLimits::try_new(1, 1, 1, 1).unwrap();

        assert_eq!(
            sequence
                .try_rebind_derivations_with_limits(&remap, &target_arena, limits)
                .unwrap_err(),
            RemapError::LimitExceeded {
                limit: RemapLimit::WorkUnits,
                maximum: 1,
            }
        );
    }

    #[test]
    fn retained_footprint_counts_spare_capacity_not_only_live_cells() {
        let (arena, derivation) = generated_derivation(61);
        let from_generation = generation();
        let to_generation = generation();
        let target = vertex(to_generation, 0);
        let mut builder = RemapBuilder::<VertexRemapSpace>::try_new(
            from_generation,
            to_generation,
            std::iter::empty(),
            [target],
        )
        .unwrap();
        builder.try_generate(target, derivation).unwrap();
        let mut envelope = builder.try_finish().unwrap();
        envelope.envelope.derivations.try_reserve_exact(64).unwrap();
        let RemapRelation::General {
            forward,
            forward_targets,
            reverse,
            reverse_origins,
        } = &mut envelope.envelope.relation
        else {
            panic!("a generated relation must use general storage");
        };
        forward.try_reserve_exact(64).unwrap();

        let logical_cells = forward.len()
            + forward_targets.len()
            + reverse.len()
            + reverse_origins.len()
            + envelope.envelope.derivations.len();
        let expected_envelope_bytes = forward.capacity()
            * std::mem::size_of::<ForwardHeader<VertexEndpoint>>()
            + forward_targets.capacity() * std::mem::size_of::<VertexEndpoint>()
            + reverse.capacity() * std::mem::size_of::<ReverseHeader<VertexEndpoint>>()
            + reverse_origins.capacity() * std::mem::size_of::<VertexEndpoint>()
            + envelope.envelope.derivations.capacity()
                * std::mem::size_of::<DerivationProvenance>();
        let envelope_footprint = envelope.retained_footprint().unwrap();
        assert!(envelope_footprint.cells() > logical_cells);
        assert_eq!(envelope_footprint.bytes(), expected_envelope_bytes);

        let mut sequence = RemapSequence::try_new([envelope]).unwrap();
        sequence.steps.try_reserve_exact(64).unwrap();
        let nested = sequence.steps[0].retained_footprint().unwrap();
        let sequence_footprint = sequence.retained_footprint().unwrap();
        assert_eq!(
            sequence_footprint.cells(),
            sequence.steps.capacity() + nested.cells()
        );
        assert_eq!(
            sequence_footprint.bytes(),
            sequence.steps.capacity() * std::mem::size_of::<RemapStep<VertexRemapSpace>>()
                + nested.bytes()
        );
        assert!(sequence.retained_allocation_cells().unwrap() > logical_cells);
        assert_eq!(
            sequence.retained_allocation_bytes().unwrap(),
            sequence_footprint.bytes()
        );
        assert!(derivation.operation_identity(&arena).is_ok());
    }

    #[test]
    fn composition_deduplicates_converged_split_paths_stably() {
        let generation_0 = generation();
        let generation_1 = generation();
        let generation_2 = generation();
        let old = vertex(generation_0, 0);
        let middle = [vertex(generation_1, 0), vertex(generation_1, 1)];
        let final_endpoint = vertex(generation_2, 0);
        let mut first =
            RemapBuilder::<VertexRemapSpace>::try_new(generation_0, generation_1, [old], middle)
                .unwrap();
        first.try_split(old, [middle[1], middle[0]]).unwrap();
        let mut second = RemapBuilder::<VertexRemapSpace>::try_new(
            generation_1,
            generation_2,
            middle,
            [final_endpoint],
        )
        .unwrap();
        second
            .try_merge([middle[0], middle[1]], final_endpoint)
            .unwrap();

        let sequence =
            RemapSequence::try_new([first.try_finish().unwrap(), second.try_finish().unwrap()])
                .unwrap();
        let composed = sequence.composed();
        assert_eq!(composed.outcome(old).unwrap().targets(), &[final_endpoint]);
        assert_eq!(composed.origins(final_endpoint).unwrap().origins(), &[old]);
    }

    #[test]
    fn sequence_checks_all_generation_boundaries_before_endpoint_lookup() {
        let generation_0 = generation();
        let generation_1 = generation();
        let foreign_generation = generation();
        let generation_3 = generation();
        let old = vertex(generation_0, 0);
        let middle = vertex(generation_1, 0);
        let foreign = vertex(foreign_generation, u32::MAX);
        let final_endpoint = vertex(generation_3, 0);

        let mut first =
            RemapBuilder::<VertexRemapSpace>::try_new(generation_0, generation_1, [old], [middle])
                .unwrap();
        first.try_map(old, middle).unwrap();
        let mut second = RemapBuilder::<VertexRemapSpace>::try_new(
            foreign_generation,
            generation_3,
            [foreign],
            [final_endpoint],
        )
        .unwrap();
        second.try_map(foreign, final_endpoint).unwrap();

        assert_eq!(
            RemapSequence::try_new([first.try_finish().unwrap(), second.try_finish().unwrap()])
                .unwrap_err(),
            RemapError::StepBoundaryMismatch {
                step: 1,
                expected: generation_1,
                actual: foreign_generation,
            }
        );
    }

    #[test]
    fn unchanged_steps_compose_as_an_explicit_identity_relation() {
        let generation_0 = generation();
        let generation_1 = generation();
        let generation_2 = generation();
        let old = NodeId::new(generation_0, 0);
        let middle = NodeId::new(generation_1, 0);
        let final_endpoint = NodeId::new(generation_2, 0);
        let first =
            RemapBuilder::<NodeRemapSpace>::try_new(generation_0, generation_1, [old], [middle])
                .unwrap()
                .try_finish_explicitly_unchanged()
                .unwrap();
        let second = RemapBuilder::<NodeRemapSpace>::try_new(
            generation_1,
            generation_2,
            [middle],
            [final_endpoint],
        )
        .unwrap()
        .try_finish_explicitly_unchanged()
        .unwrap();
        let sequence = RemapSequence::try_new([first, second]).unwrap();
        assert!(sequence.composed().is_explicitly_unchanged());
        assert!(matches!(
            &sequence.composed().relation,
            RemapRelation::ExplicitlyUnchanged { inputs, outputs }
                if inputs == &[old] && outputs == &[final_endpoint]
        ));
        assert_eq!(sequence.composed().retained_footprint().unwrap().cells(), 2);
        assert_eq!(
            sequence.composed().outcome(old).unwrap().targets(),
            &[final_endpoint]
        );
    }

    #[test]
    fn unchanged_composition_reports_missing_to_before_missing_from() {
        let generation_0 = generation();
        let generation_1 = generation();
        let generation_2 = generation();
        let old = NodeId::new(generation_0, 1);
        let missing_to = NodeId::new(generation_1, 1);
        let first = RemapBuilder::<NodeRemapSpace>::try_new(
            generation_0,
            generation_1,
            [old],
            [missing_to],
        )
        .unwrap()
        .try_finish_explicitly_unchanged()
        .unwrap();
        let right_inputs = [NodeId::new(generation_1, 0), NodeId::new(generation_1, 2)];
        let right_outputs = [NodeId::new(generation_2, 0), NodeId::new(generation_2, 2)];
        let second = RemapBuilder::<NodeRemapSpace>::try_new(
            generation_1,
            generation_2,
            right_inputs,
            right_outputs,
        )
        .unwrap()
        .try_finish_explicitly_unchanged()
        .unwrap();

        assert_eq!(
            RemapSequence::try_new([first, second]).unwrap_err(),
            RemapError::MissingIntermediateEndpoint {
                side: RemapSide::To,
                endpoint: missing_to,
            }
        );
    }

    #[test]
    fn limits_work_and_allocation_failures_are_typed() {
        let from_generation = generation();
        let to_generation = generation();
        let old = [vertex(from_generation, 0), vertex(from_generation, 1)];
        let new = [vertex(to_generation, 0), vertex(to_generation, 1)];
        let endpoint_limit = RemapLimits::try_new(1, 2, 2, 100).unwrap();
        assert_eq!(
            RemapBuilder::<VertexRemapSpace>::try_new_with_limits(
                from_generation,
                to_generation,
                old,
                new,
                endpoint_limit,
            )
            .unwrap_err(),
            RemapError::LimitExceeded {
                limit: RemapLimit::EndpointsPerSide,
                maximum: 1,
            }
        );

        let relation_limit = RemapLimits::try_new(2, 1, 2, 100).unwrap();
        let mut builder = RemapBuilder::<VertexRemapSpace>::try_new_with_limits(
            from_generation,
            to_generation,
            [old[0]],
            new,
            relation_limit,
        )
        .unwrap();
        assert_eq!(
            builder.try_split(old[0], new),
            Err(RemapError::LimitExceeded {
                limit: RemapLimit::RelationCells,
                maximum: 1,
            })
        );

        let (_arena, derivation) = generated_derivation(21);
        let derivation_limit = RemapLimits::try_new(1, 0, 1, 100).unwrap();
        let mut generated = RemapBuilder::<VertexRemapSpace>::try_new_with_limits(
            from_generation,
            to_generation,
            std::iter::empty(),
            [new[0]],
            derivation_limit,
        )
        .unwrap();
        assert_eq!(
            generated.try_generate(new[0], derivation),
            Err(RemapError::LimitExceeded {
                limit: RemapLimit::RelationCells,
                maximum: 0,
            })
        );

        let work_limit = RemapLimits::try_new(1, 1, 1, 0).unwrap();
        assert_eq!(
            RemapBuilder::<VertexRemapSpace>::try_new_with_limits(
                from_generation,
                to_generation,
                [old[0]],
                [new[0]],
                work_limit,
            )
            .unwrap_err(),
            RemapError::LimitExceeded {
                limit: RemapLimit::WorkUnits,
                maximum: 0,
            }
        );

        let finish_work_limit = RemapLimits::try_new(1, 1, 1, 9).unwrap();
        let mut finish_limited = RemapBuilder::<VertexRemapSpace>::try_new_with_limits(
            from_generation,
            to_generation,
            [old[0]],
            [new[0]],
            finish_work_limit,
        )
        .unwrap();
        finish_limited.try_map(old[0], new[0]).unwrap();
        assert_eq!(
            finish_limited.try_finish().unwrap_err(),
            RemapError::LimitExceeded {
                limit: RemapLimit::WorkUnits,
                maximum: 9,
            },
            "completeness and envelope construction must remain metered"
        );

        let mut bytes = Vec::<u8>::new();
        assert_eq!(
            try_reserve_vec::<u8, VertexEndpoint>(
                &mut bytes,
                usize::MAX,
                RemapAllocation::ForwardTargets,
            )
            .unwrap_err(),
            RemapError::AllocationFailed {
                resource: RemapAllocation::ForwardTargets,
            }
        );
    }

    #[test]
    fn radix_canonicalization_spends_each_pass_before_reordering() {
        let generation = generation();
        let mut endpoints = vec![vertex(generation, 1), vertex(generation, 0)];
        let original = endpoints.clone();
        let mut work = WorkCounter::new(endpoints.len());

        assert_eq!(
            canonicalize_endpoints(&mut endpoints, &mut work),
            Err(RemapError::LimitExceeded {
                limit: RemapLimit::WorkUnits,
                maximum: original.len(),
            })
        );
        assert_eq!(work.used, original.len());
        assert_eq!(endpoints, original);
    }

    #[test]
    fn sequence_limits_and_empty_sequences_fail_without_partial_composition() {
        assert_eq!(
            RemapSequence::<VertexRemapSpace>::try_new(std::iter::empty()).unwrap_err(),
            RemapError::EmptySequence
        );

        let generation_0 = generation();
        let generation_1 = generation();
        let old = vertex(generation_0, 0);
        let new = vertex(generation_1, 0);
        let mut builder =
            RemapBuilder::<VertexRemapSpace>::try_new(generation_0, generation_1, [old], [new])
                .unwrap();
        builder.try_map(old, new).unwrap();
        let limits = RemapLimits::try_new(1, 1, 0, 10).unwrap();
        assert_eq!(
            RemapSequence::try_new_with_limits([builder.try_finish().unwrap()], limits)
                .unwrap_err(),
            RemapError::LimitExceeded {
                limit: RemapLimit::Steps,
                maximum: 0,
            }
        );

        let mut builder =
            RemapBuilder::<VertexRemapSpace>::try_new(generation_0, generation_1, [old], [new])
                .unwrap();
        builder.try_map(old, new).unwrap();
        let work_limits = RemapLimits::try_new(1, 1, 1, 0).unwrap();
        assert_eq!(
            RemapSequence::try_new_with_limits([builder.try_finish().unwrap()], work_limits)
                .unwrap_err(),
            RemapError::LimitExceeded {
                limit: RemapLimit::WorkUnits,
                maximum: 0,
            },
            "step collection itself must spend bounded work"
        );
    }

    #[test]
    fn composition_meters_headers_even_when_every_relation_is_empty() {
        let generation_0 = generation();
        let generation_1 = generation();
        let generation_2 = generation();
        let old = vertex(generation_0, 0);
        let mut first = RemapBuilder::<VertexRemapSpace>::try_new(
            generation_0,
            generation_1,
            [old],
            std::iter::empty(),
        )
        .unwrap();
        first.try_delete(old).unwrap();
        let first = first.try_finish().unwrap();
        let second = RemapBuilder::<VertexRemapSpace>::try_new(
            generation_1,
            generation_2,
            std::iter::empty(),
            std::iter::empty(),
        )
        .unwrap()
        .try_finish_explicitly_unchanged()
        .unwrap();
        let limits = RemapLimits::try_new(1, 0, 2, 3).unwrap();

        assert_eq!(
            RemapSequence::try_new_with_limits([first, second], limits).unwrap_err(),
            RemapError::LimitExceeded {
                limit: RemapLimit::WorkUnits,
                maximum: 3,
            }
        );
    }

    #[test]
    fn custom_limits_cannot_exceed_absolute_bounds() {
        assert_eq!(
            RemapLimits::try_new(
                MAX_REMAP_ENDPOINTS_PER_SIDE + 1,
                MAX_REMAP_RELATION_CELLS,
                MAX_REMAP_STEPS,
                MAX_REMAP_WORK_UNITS,
            ),
            Err(RemapLimitsError::ExceedsHardLimit {
                limit: RemapLimit::EndpointsPerSide,
                requested: MAX_REMAP_ENDPOINTS_PER_SIDE + 1,
                maximum: MAX_REMAP_ENDPOINTS_PER_SIDE,
            })
        );
    }
}
