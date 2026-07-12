use crate::{
    PostProcessDimension, PostProcessLedgerPhase, PostProcessLedgerSnapshot, PostProcessStep,
};
use baozi_core::{PrimitiveTopology, SceneValidationError, ValidatedScene};
use baozi_operation::{
    ControlCause, LedgerCause, LedgerFinishError, LedgerLimitsError, PublicationCause,
};
use std::error::Error;
use std::fmt;
use thiserror::Error;

/// A bounded failure while compiling a post-process step request.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PostProcessPlanError {
    /// The request kept producing duplicate or otherwise redundant entries past the hard bound.
    #[error("post-process plan contains more than {maximum} requested entries")]
    TooManyRequestedEntries { maximum: usize },
}

/// Allocation-free context for a failed post-process transformation.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PostProcessStepIssue {
    /// A mesh disappeared while an edit was being applied.
    #[error("mesh {mesh} disappeared while applying the step")]
    MeshMissing { mesh: usize },

    /// An implicit polygon index cannot be represented by the scene index type.
    #[error("mesh {mesh} implicit polygon index {index} exceeds u32 range")]
    ImplicitIndexOutOfRange { mesh: usize, index: usize },

    /// A polygon corner count cannot be represented by the host collection index type.
    #[error("mesh {mesh} face {face} corner count exceeds usize range")]
    PolygonCornerCountOutOfRange { mesh: usize, face: usize },

    /// The number of indices required by triangulation overflowed.
    #[error("mesh {mesh} triangle index count overflowed")]
    TriangleIndexCountOverflow { mesh: usize },

    /// A polygon face range overflowed while its end offset was computed.
    #[error("mesh {mesh} face {face} range overflowed")]
    PolygonFaceRangeOverflow { mesh: usize, face: usize },

    /// A polygon face range extends beyond the available source elements.
    #[error("mesh {mesh} face {face} ends at element {end}, beyond {available} source elements")]
    PolygonFaceRangeInvalid {
        mesh: usize,
        face: usize,
        end: usize,
        available: usize,
    },

    /// A polygon face contains no anchor corner.
    #[error("mesh {mesh} face {face} is empty")]
    PolygonFaceEmpty { mesh: usize, face: usize },

    /// A declared polygon corner is absent from the source elements.
    #[error("mesh {mesh} face {face} corner {corner} is missing")]
    PolygonCornerMissing {
        mesh: usize,
        face: usize,
        corner: usize,
    },

    /// Polygon face counts did not consume the complete source-element sequence.
    #[error("mesh {mesh} face counts consume {consumed} of {available} source elements")]
    PolygonFaceCountsMismatch {
        mesh: usize,
        consumed: usize,
        available: usize,
    },

    /// Normal generation requires triangle topology.
    #[error("mesh {mesh} has {topology:?} topology; triangulate before generating normals")]
    RequiresTriangles {
        mesh: usize,
        topology: PrimitiveTopology,
    },

    /// A triangle did not contain exactly three positions or indices.
    #[error("mesh {mesh} triangle {triangle} has invalid arity (indexed={indexed})")]
    TriangleArityMismatch {
        mesh: usize,
        triangle: usize,
        indexed: bool,
    },

    /// A generated-normal vertex offset overflowed.
    #[error("mesh {mesh} triangle {triangle} vertex index overflowed")]
    VertexIndexOverflow { mesh: usize, triangle: usize },

    /// A triangle references a vertex outside the available vertex sequence.
    #[error(
        "mesh {mesh} triangle {triangle} vertex {vertex} is outside the {upper_bound}-vertex range"
    )]
    VertexIndexOutOfRange {
        mesh: usize,
        triangle: usize,
        vertex: usize,
        upper_bound: usize,
    },

    /// A triangle cannot produce a finite, non-zero normal.
    #[error("mesh {mesh} triangle {triangle} is degenerate")]
    DegenerateTriangle { mesh: usize, triangle: usize },

    /// A vertex is not referenced by any triangle.
    #[error("mesh {mesh} vertex {vertex} is not referenced by any triangle")]
    UnreferencedVertex { mesh: usize, vertex: usize },

    /// A shared vertex would require incompatible generated normals.
    #[error("mesh {mesh} shared vertex {vertex} needs multiple generated normals")]
    ConflictingGeneratedNormal { mesh: usize, vertex: usize },

    /// The byte count required for a staging allocation overflowed `usize`.
    #[error(
        "{element_count} staging elements of {element_size} bytes overflow the capacity byte count"
    )]
    CapacityByteCountOverflow {
        element_count: usize,
        element_size: usize,
    },

    /// A staging byte count cannot be represented by operation accounting.
    #[error("staging capacity {bytes} exceeds u64 range")]
    CapacityExceedsU64 { bytes: usize },
}

/// A typed post-process domain error.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PostProcessError {
    /// A requested transformation could not produce a valid result.
    #[error("post-process step {step:?} failed: {issue}")]
    StepFailed {
        step: PostProcessStep,
        #[source]
        issue: PostProcessStepIssue,
    },

    /// Final central Scene validation rejected the staged edit.
    #[error("post-process output validation failed")]
    Validation {
        #[source]
        source: SceneValidationError,
    },

    /// Cooperative operation control stopped the invocation.
    #[error("post-process operation stopped")]
    Control {
        #[source]
        source: ControlCause,
    },

    /// Checked operation accounting rejected work or staging capacity.
    #[error("post-process resource accounting failed")]
    Accounting {
        #[source]
        source: LedgerCause<PostProcessDimension>,
    },

    /// A future owner-neutral publication failure not covered by a known category.
    #[error("post-process publication failed")]
    Publication {
        #[source]
        source: PublicationCause<PostProcessDimension>,
    },

    /// Operation-ledger finalization failed after the pipeline reached a terminal state.
    #[error("post-process ledger finalization failed")]
    LedgerFinalization,

    /// A staging allocation could not be reserved.
    #[error("post-process {context} could not reserve {resource}")]
    CapacityExceeded {
        context: &'static str,
        resource: &'static str,
    },

    /// Core supplied an invalid reservation transition to the owner adapter.
    #[error("post-process validation-control invariant failed: {message}")]
    ValidationControlInvariant { message: &'static str },
}

impl PostProcessError {
    pub(crate) const fn step(step: PostProcessStep, issue: PostProcessStepIssue) -> Self {
        Self::StepFailed { step, issue }
    }
}

/// Borrowed evidence for a ledger failure observed during post-process finalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostProcessFinalizationError<'a> {
    cause: &'a LedgerCause<PostProcessDimension>,
    snapshot: &'a PostProcessLedgerSnapshot,
}

impl<'a> PostProcessFinalizationError<'a> {
    const fn new(
        cause: &'a LedgerCause<PostProcessDimension>,
        snapshot: &'a PostProcessLedgerSnapshot,
    ) -> Self {
        Self { cause, snapshot }
    }

    /// Returns the invariant that prevented clean finalization.
    pub const fn cause(self) -> &'a LedgerCause<PostProcessDimension> {
        self.cause
    }

    /// Returns the exact non-normalized evidence observed at finalization.
    pub const fn snapshot(self) -> &'a PostProcessLedgerSnapshot {
        self.snapshot
    }
}

impl fmt::Display for PostProcessFinalizationError<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "post-process ledger finalization failed: {}",
            self.cause
        )
    }
}

impl Error for PostProcessFinalizationError<'_> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause)
    }
}

/// A failed post-process invocation with immutable resource evidence.
///
/// The input scene is borrowed by the pipeline and therefore remains usable by
/// the caller. Clean finalization retains the finalized snapshot; an invariant
/// failure retains the exact finalization cause and its owned snapshot without
/// duplicating either value.
#[derive(Debug, PartialEq, Eq)]
pub struct PostProcessFailure {
    error: PostProcessError,
    ledger: PostProcessLedgerSnapshot,
    finalization_cause: Option<LedgerCause<PostProcessDimension>>,
}

impl PostProcessFailure {
    pub(crate) fn new(error: PostProcessError, ledger: PostProcessLedgerSnapshot) -> Self {
        Self {
            error,
            ledger,
            finalization_cause: None,
        }
    }

    pub(crate) fn with_finalization_error(
        error: PostProcessError,
        finalization_error: LedgerFinishError<PostProcessLedgerPhase, PostProcessDimension>,
    ) -> Self {
        let (finalization_cause, ledger) = finalization_error.into_parts();
        Self {
            error,
            ledger,
            finalization_cause: Some(finalization_cause),
        }
    }

    /// Returns the typed domain error.
    pub fn error(&self) -> &PostProcessError {
        &self.error
    }

    /// Returns immutable operation-ledger evidence.
    pub fn ledger(&self) -> &PostProcessLedgerSnapshot {
        &self.ledger
    }

    /// Returns borrowed finalization cause and snapshot evidence, when cleanup itself failed.
    pub fn finalization_error(&self) -> Option<PostProcessFinalizationError<'_>> {
        self.finalization_cause
            .as_ref()
            .map(|cause| PostProcessFinalizationError::new(cause, &self.ledger))
    }

    /// Consumes the failure and returns the domain error, final snapshot, and
    /// optional ledger-finalization cause.
    pub fn into_parts(
        self,
    ) -> (
        PostProcessError,
        PostProcessLedgerSnapshot,
        Option<LedgerCause<PostProcessDimension>>,
    ) {
        (self.error, self.ledger, self.finalization_cause)
    }
}

impl fmt::Display for PostProcessFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl Error for PostProcessFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match (&self.error, self.finalization_cause.as_ref()) {
            (PostProcessError::LedgerFinalization, Some(cause)) => Some(cause),
            _ => Some(&self.error),
        }
    }
}

/// A failure before or during one post-process invocation.
///
/// Initialization failures occur before an operation coordinator exists and
/// therefore cannot carry a ledger snapshot. Once initialization succeeds,
/// every failure is retained as a complete [`PostProcessFailure`].
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum PostProcessRunError {
    /// The operation ledger policy could not be frozen.
    #[error("post-process operation initialization failed: {source}")]
    Initialization {
        #[source]
        source: LedgerLimitsError<PostProcessDimension>,
    },

    /// The initialized operation failed and retained its terminal evidence.
    #[error("post-process operation failed: {failure}")]
    Operation {
        #[source]
        failure: PostProcessFailure,
    },
}

impl PostProcessRunError {
    pub(crate) const fn initialization(source: LedgerLimitsError<PostProcessDimension>) -> Self {
        Self::Initialization { source }
    }

    pub(crate) const fn operation(failure: PostProcessFailure) -> Self {
        Self::Operation { failure }
    }

    /// Extracts the complete operation failure without discarding an
    /// initialization error on mismatch.
    pub fn into_operation_failure(
        self,
    ) -> std::result::Result<PostProcessFailure, LedgerLimitsError<PostProcessDimension>> {
        match self {
            Self::Operation { failure } => Ok(failure),
            Self::Initialization { source } => Err(source),
        }
    }
}

/// A durably published post-process generation and its immutable operation evidence.
#[derive(Debug, PartialEq)]
pub struct PostProcessSuccess {
    scene: ValidatedScene,
    ledger: PostProcessLedgerSnapshot,
}

impl PostProcessSuccess {
    pub(crate) const fn new(scene: ValidatedScene, ledger: PostProcessLedgerSnapshot) -> Self {
        Self { scene, ledger }
    }

    /// Returns the durably published validated generation.
    pub const fn scene(&self) -> &ValidatedScene {
        &self.scene
    }

    /// Returns immutable operation-ledger evidence.
    pub const fn ledger(&self) -> &PostProcessLedgerSnapshot {
        &self.ledger
    }

    /// Consumes the report and returns the durably published scene.
    pub fn into_scene(self) -> ValidatedScene {
        self.scene
    }

    /// Consumes the report and returns all durable success evidence.
    pub fn into_parts(self) -> (ValidatedScene, PostProcessLedgerSnapshot) {
        (self.scene, self.ledger)
    }
}

/// The result of a post-process invocation.
pub type Result<T> = std::result::Result<T, PostProcessRunError>;

pub(crate) type StepResult<T> = std::result::Result<T, PostProcessError>;
