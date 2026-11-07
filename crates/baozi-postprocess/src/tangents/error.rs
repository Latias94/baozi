use super::TangentIssue;
use crate::direct_error::FailureOwner;
use crate::error::project_control_cause;
use crate::{PostProcessControlCause, PostProcessResourceEvidence};
use baozi_core::SceneLookupError;
use baozi_operation::{ControlCause, StoreRuntimeError};
use std::fmt;
use thiserror::Error;

/// Why one direct tangent generation could not publish an output.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TangentCause {
    #[error("the target primitive could not be looked up: {source}")]
    Target {
        #[source]
        source: SceneLookupError,
    },
    #[error("tangent generation requires TriangleList topology")]
    UnsupportedTopology,
    #[error("the validated primitive has no supported vertex POSITION stream")]
    Positions,
    #[error("the selected NORMAL domain is absent or unsupported")]
    NormalField,
    #[error("the selected TEXCOORD set/domain is absent or unsupported")]
    UvField,
    #[error("tangent generation supports Vertex or Corner fields")]
    UnsupportedDomain,
    #[error(
        "tangent generation cannot choose among multiple authored TANGENT domains for replacement"
    )]
    MultipleTangentFields,
    #[error("the base-only tangent profile cannot preserve authored morph deltas")]
    MorphDeltas,
    #[error("tangent generation failed: {issue:?}")]
    Issue { issue: TangentIssue },
    #[error("tangent generation stopped: {cause}")]
    Control {
        #[source]
        cause: PostProcessControlCause,
    },
    #[error("operation control stopped final publication")]
    FinalControl,
    #[error("tangent generation storage or work admission failed: {source}")]
    Runtime {
        #[source]
        source: StoreRuntimeError,
    },
    #[error("the core rejected the geometry edit")]
    CoreRejected,
    #[error("tangent generation would narrow its {semantic:?} precision dependency")]
    PrecisionLoss {
        semantic: baozi_core::AttributeSemanticKind,
    },
    #[error("central validation rejected the geometry edit")]
    CentralValidation,
    #[error("the geometry edit could not acquire or bind its final generation")]
    Publication,
}

impl From<StoreRuntimeError> for TangentCause {
    fn from(source: StoreRuntimeError) -> Self {
        match source {
            StoreRuntimeError::Control(cause) => Self::control(cause),
            source => Self::Runtime { source },
        }
    }
}

impl TangentCause {
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
pub struct TangentFailure {
    cause: TangentCause,
    resources: Option<PostProcessResourceEvidence>,
    _owner: FailureOwner,
}

impl TangentFailure {
    pub(crate) const fn new(cause: TangentCause) -> Self {
        Self {
            cause,
            resources: None,
            _owner: FailureOwner::None,
        }
    }

    pub(crate) fn terminal(
        cause: TangentCause,
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
    pub const fn cause(&self) -> &TangentCause {
        &self.cause
    }

    /// Returns real terminal resource evidence; absent for startup failure and fatal containment.
    pub const fn resources(&self) -> Option<&PostProcessResourceEvidence> {
        self.resources.as_ref()
    }
}

impl fmt::Debug for TangentFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TangentFailure")
            .field("cause", &self.cause)
            .field("resources", &self.resources)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for TangentFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(formatter)
    }
}

impl std::error::Error for TangentFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}
impl From<crate::scene_edit::EditSemanticFailure> for TangentFailure {
    fn from(failure: crate::scene_edit::EditSemanticFailure) -> Self {
        use crate::scene_edit::EditCause;
        let cause = match failure.cause {
            EditCause::Runtime { source } => TangentCause::Runtime { source },
            EditCause::Control { cause } => TangentCause::Control { cause },
            EditCause::FinalControl => TangentCause::FinalControl,
            EditCause::CoreRejected => TangentCause::CoreRejected,
            EditCause::PrecisionLoss { semantic } => TangentCause::PrecisionLoss { semantic },
            EditCause::MorphDependency { .. } => TangentCause::MorphDeltas,
            EditCause::SpaceIssue { .. } => TangentCause::CoreRejected,
            EditCause::CentralValidation => TangentCause::CentralValidation,
            EditCause::Publication => TangentCause::Publication,
        };
        Self {
            cause,
            resources: failure.resources,
            _owner: failure.owner,
        }
    }
}

impl From<super::input::InputError> for TangentCause {
    fn from(error: super::input::InputError) -> Self {
        use super::input::InputError;
        match error {
            InputError::Positions => Self::Positions,
            InputError::NormalField => Self::NormalField,
            InputError::UvField => Self::UvField,
            InputError::UnsupportedDomain => Self::UnsupportedDomain,
            InputError::Runtime(source) => source.into(),
            InputError::Issue(issue) => Self::Issue { issue },
        }
    }
}
