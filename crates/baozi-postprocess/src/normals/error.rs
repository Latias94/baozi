use crate::direct_error::FailureOwner;
use crate::error::project_control_cause;
use crate::{PostProcessControlCause, PostProcessResourceEvidence};
use baozi_core::SceneLookupError;
use baozi_operation::__retained_store_integration::activation::StoreActivationContainment;
use baozi_operation::__retained_store_integration::authority::RetainedStoreStartError;
use baozi_operation::{ControlCause, StorePrepareError, StoreRuntimeError};
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
    InvalidKernelShape,
}

/// Why one direct normal generation could not publish an output.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NormalCause {
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
    #[error("normal generation failed: {issue:?}")]
    Issue { issue: NormalIssue },
    #[error("normal generation size or work bound overflowed")]
    SizeOverflow,
    #[error("normal generation requires {required} work units, above the {maximum} unit limit")]
    WorkLimit { required: u64, maximum: u64 },
    #[error("normal generation stopped: {cause}")]
    Control {
        #[source]
        cause: PostProcessControlCause,
    },
    #[error("operation control stopped final publication")]
    FinalControl,
    #[error("the same-account edit operation could not start: {source}")]
    Start {
        #[source]
        source: RetainedStoreStartError,
    },
    #[error("normal generation storage or work admission failed: {source}")]
    Runtime {
        #[source]
        source: StoreRuntimeError,
    },
    #[error("the core rejected the geometry edit")]
    CoreRejected,
    #[error("central validation rejected the geometry edit")]
    CentralValidation,
    #[error("the geometry edit could not acquire or bind its final generation")]
    Publication,
    #[error("the geometry edit requires fail-closed containment: {source}")]
    Containment {
        #[source]
        source: StoreActivationContainment,
    },
    #[error("final geometry-edit preparation requires containment: {source}")]
    Preparation {
        #[source]
        source: StorePrepareError,
    },
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
impl From<crate::geometry_edit::EditFailure> for NormalFailure {
    fn from(failure: crate::geometry_edit::EditFailure) -> Self {
        use crate::geometry_edit::EditCause;
        let cause = match failure.cause {
            EditCause::Runtime { source } => NormalCause::Runtime { source },
            EditCause::Control { cause } => NormalCause::Control { cause },
            EditCause::FinalControl => NormalCause::FinalControl,
            EditCause::CoreRejected => NormalCause::CoreRejected,
            EditCause::CentralValidation => NormalCause::CentralValidation,
            EditCause::Publication => NormalCause::Publication,
            EditCause::Containment { source } => NormalCause::Containment { source },
            EditCause::Preparation { source } => NormalCause::Preparation { source },
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
            InputError::Target(source) => Self::Target { source },
            InputError::UnsupportedTopology => Self::UnsupportedTopology,
            InputError::Positions => Self::Positions,
            InputError::SmoothingGroups => Self::SmoothingGroups,
            InputError::TangentsRequireRemoval => Self::TangentsRequireRemoval,
            InputError::MultipleNormalFields => Self::MultipleNormalFields,
            InputError::MultipleTangentFields => Self::MultipleTangentFields,
            InputError::SizeOverflow => Self::SizeOverflow,
            InputError::WorkLimit { required, maximum } => Self::WorkLimit { required, maximum },
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
                    _ => NormalIssue::InvalidKernelShape,
                },
            },
        }
    }
}
