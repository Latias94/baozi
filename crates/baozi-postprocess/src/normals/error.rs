use crate::direct_error::FailureOwner;
use crate::error::project_control_cause;
use crate::{PostProcessControlCause, PostProcessResourceEvidence};
use baozi_core::SceneLookupError;
use baozi_operation::{ControlCause, StoreRuntimeError};
use std::fmt;
use thiserror::Error;

/// A numerical or topology failure detected while generating a normal field.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NormalIssue {
    NonFinitePosition {
        vertex: usize,
        component: usize,
    },
    VertexOutOfBounds {
        corner: usize,
        vertex: usize,
        vertex_count: usize,
    },
    DegenerateFace {
        face: usize,
    },
    InvalidFaceCross {
        face: usize,
    },
    InvalidSmoothSum {
        vertex: usize,
        group: Option<u32>,
        corner: usize,
    },
    /// Binary64 scaling failed at an original Position component.
    PositionNumerical {
        vertex: usize,
        component: usize,
        issue: crate::GeometryNumericIssue,
    },
    /// Binary64 face estimation, normalization, or output validation failed.
    Numerical {
        face: usize,
        issue: crate::GeometryNumericIssue,
    },
    /// Binary64 area accumulation or normalization failed in a smoothing bucket.
    SmoothNumerical {
        vertex: usize,
        group: Option<u32>,
        /// Lowest original corner in the bucket, before deterministic sorting.
        corner: usize,
        issue: crate::GeometryNumericIssue,
    },
    InvalidKernelShape,
}

/// Why one direct normal generation could not publish an output.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NormalCause {
    #[error("the explicit tangent preparation failed: {cause}")]
    Tangent {
        #[source]
        cause: crate::TangentCause,
    },
    #[error("the target primitive could not be looked up: {source}")]
    Target {
        #[source]
        source: SceneLookupError,
    },
    #[error("normal generation requires TriangleList topology")]
    UnsupportedTopology,
    #[error("the validated primitive has no supported vertex POSITION stream")]
    Positions,
    #[error("the validated primitive has no supported Face smoothing-group stream")]
    SmoothingGroups,
    #[error("generating normals requires explicit removal of the invalidated TANGENT field")]
    TangentsRequireRemoval,
    #[error("normal generation cannot select or merge multiple authored NORMAL domains")]
    MultipleNormalFields,
    #[error("normal generation cannot remove multiple authored TANGENT domains")]
    MultipleTangentFields,
    #[error("normal generation would invalidate authored {semantic:?} morph deltas")]
    MorphDeltas {
        semantic: baozi_core::MorphDeltaSemantic,
    },
    #[error("normal generation failed: {issue:?}")]
    Issue { issue: NormalIssue },
    #[error("normal generation size or work bound overflowed")]
    SizeOverflow,
    #[error("normal generation stopped: {cause}")]
    Control {
        #[source]
        cause: PostProcessControlCause,
    },
    #[error("operation control stopped final publication")]
    FinalControl,
    #[error("normal generation storage or work admission failed: {source}")]
    Runtime {
        #[source]
        source: StoreRuntimeError,
    },
    #[error("the core rejected the geometry edit")]
    CoreRejected,
    #[error("normal generation would narrow its {semantic:?} precision dependency")]
    PrecisionLoss {
        semantic: baozi_core::AttributeSemanticKind,
    },
    #[error("central validation rejected the geometry edit")]
    CentralValidation,
    #[error("the geometry edit could not acquire or bind its final generation")]
    Publication,
}

impl From<StoreRuntimeError> for NormalCause {
    fn from(source: StoreRuntimeError) -> Self {
        match source {
            StoreRuntimeError::Control(cause) => Self::control(cause),
            source => Self::Runtime { source },
        }
    }
}

impl NormalCause {
    pub(crate) fn control(cause: ControlCause) -> Self {
        Self::Control {
            cause: project_control_cause(cause),
        }
    }
}

/// A typed direct-operation error and the actual published terminal evidence, when available.
///
/// Startup and fatal containment have no fabricated ordinary terminal resource snapshot.
/// Invocation failures are returned by the outer run boundary with their original owner.
pub struct NormalFailure {
    cause: NormalCause,
    resources: Option<PostProcessResourceEvidence>,
    _owner: FailureOwner,
}

impl NormalFailure {
    pub(crate) const fn new(cause: NormalCause) -> Self {
        Self {
            cause,
            resources: None,
            _owner: FailureOwner::None,
        }
    }

    pub(crate) fn terminal(
        cause: NormalCause,
        resources: PostProcessResourceEvidence,
        owner: FailureOwner,
    ) -> Self {
        Self {
            cause,
            resources: Some(resources),
            _owner: owner,
        }
    }

    /// Returns the primary rejection or exact containment cause.
    pub const fn cause(&self) -> &NormalCause {
        &self.cause
    }

    /// Returns real terminal resource evidence; absent for startup failure and fatal containment.
    pub const fn resources(&self) -> Option<&PostProcessResourceEvidence> {
        self.resources.as_ref()
    }
}

impl fmt::Debug for NormalFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NormalFailure")
            .field("cause", &self.cause)
            .field("resources", &self.resources)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for NormalFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(formatter)
    }
}

impl std::error::Error for NormalFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}
impl From<crate::scene_edit::EditSemanticFailure> for NormalFailure {
    fn from(failure: crate::scene_edit::EditSemanticFailure) -> Self {
        use crate::scene_edit::EditCause;
        let cause = match failure.cause {
            EditCause::Runtime { source } => NormalCause::Runtime { source },
            EditCause::Control { cause } => NormalCause::Control { cause },
            EditCause::FinalControl => NormalCause::FinalControl,
            EditCause::CoreRejected => NormalCause::CoreRejected,
            EditCause::PrecisionLoss { semantic } => NormalCause::PrecisionLoss { semantic },
            EditCause::MorphDependency { semantic } => NormalCause::MorphDeltas { semantic },
            EditCause::SpaceIssue { .. } => NormalCause::CoreRejected,
            EditCause::CentralValidation => NormalCause::CentralValidation,
            EditCause::Publication => NormalCause::Publication,
        };
        Self {
            cause,
            resources: failure.resources,
            _owner: failure.owner,
        }
    }
}

impl From<super::input::InputError> for NormalCause {
    fn from(error: super::input::InputError) -> Self {
        use super::{input::InputError, kernel::Error};
        match error {
            InputError::Runtime(source) | InputError::Kernel(Error::Control(source)) => {
                source.into()
            }
            InputError::Kernel(error) => Self::Issue {
                issue: match error {
                    Error::NonFinitePosition { vertex, component } => {
                        NormalIssue::NonFinitePosition { vertex, component }
                    }
                    Error::VertexOutOfBounds {
                        corner,
                        vertex,
                        vertex_count,
                    } => NormalIssue::VertexOutOfBounds {
                        corner,
                        vertex,
                        vertex_count,
                    },
                    Error::DegenerateFace { face } => NormalIssue::DegenerateFace { face },
                    Error::InvalidFaceCross { face } => NormalIssue::InvalidFaceCross { face },
                    Error::InvalidSmoothSum {
                        vertex,
                        group,
                        corner,
                    } => NormalIssue::InvalidSmoothSum {
                        vertex,
                        group,
                        corner,
                    },
                    Error::PositionNumerical {
                        vertex,
                        component,
                        issue,
                    } => NormalIssue::PositionNumerical {
                        vertex,
                        component,
                        issue,
                    },
                    Error::Numerical { face, issue } => NormalIssue::Numerical { face, issue },
                    Error::SmoothNumerical {
                        vertex,
                        group,
                        corner,
                        issue,
                    } => NormalIssue::SmoothNumerical {
                        vertex,
                        group,
                        corner,
                        issue,
                    },
                    _ => NormalIssue::InvalidKernelShape,
                },
            },
        }
    }
}
