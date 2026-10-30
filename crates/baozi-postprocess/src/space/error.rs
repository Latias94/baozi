use std::fmt;

use baozi_core::SourceSpaceError;
use baozi_operation::__retained_store_integration::activation::StoreActivationContainment;
use baozi_operation::__retained_store_integration::authority::RetainedStoreStartError;
use baozi_operation::{StorePrepareError, StoreRuntimeError};
use thiserror::Error;

use super::SpaceConversionIssue;
use crate::direct_error::FailureOwner;
use crate::{PostProcessControlCause, PostProcessResourceEvidence};

/// Why a complete Scene conversion could not publish.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SpaceConversionCause {
    #[error("source conventions cannot define the conversion: {source}")]
    Source {
        #[source]
        source: SourceSpaceError,
    },
    #[error("scene conversion cannot preserve a present semantic: {issue}")]
    Issue {
        #[source]
        issue: SpaceConversionIssue,
    },
    #[error("the same-account edit operation could not start: {source}")]
    Start {
        #[source]
        source: RetainedStoreStartError,
    },
    #[error("scene conversion storage or work admission failed: {source}")]
    Runtime {
        #[source]
        source: StoreRuntimeError,
    },
    #[error("scene conversion stopped: {cause}")]
    Control {
        #[source]
        cause: PostProcessControlCause,
    },
    #[error("operation control stopped final publication")]
    FinalControl,
    #[error("the core rejected the Scene edit")]
    CoreRejected,
    #[error("central validation rejected the Scene edit")]
    CentralValidation,
    #[error("the Scene edit could not acquire or bind its final generation")]
    Publication,
    #[error("the Scene edit requires fail-closed containment: {source}")]
    Containment {
        #[source]
        source: StoreActivationContainment,
    },
    #[error("final Scene-edit preparation requires containment: {source}")]
    Preparation {
        #[source]
        source: StorePrepareError,
    },
}

impl From<StoreRuntimeError> for SpaceConversionCause {
    fn from(source: StoreRuntimeError) -> Self {
        match source {
            StoreRuntimeError::Control(cause) => Self::Control {
                cause: crate::error::project_control_cause(cause),
            },
            source => Self::Runtime { source },
        }
    }
}

/// Typed failure and the actual retained terminal evidence, when startup and cleanup permit it.
pub struct SpaceConversionFailure {
    cause: SpaceConversionCause,
    resources: Option<PostProcessResourceEvidence>,
    _owner: FailureOwner,
}

impl SpaceConversionFailure {
    pub(crate) const fn new(cause: SpaceConversionCause) -> Self {
        Self {
            cause,
            resources: None,
            _owner: FailureOwner::None,
        }
    }
    pub(crate) fn terminal(
        cause: SpaceConversionCause,
        resources: PostProcessResourceEvidence,
        owner: FailureOwner,
    ) -> Self {
        Self {
            cause,
            resources: Some(resources),
            _owner: owner,
        }
    }
    pub const fn cause(&self) -> &SpaceConversionCause {
        &self.cause
    }
    pub const fn resources(&self) -> Option<&PostProcessResourceEvidence> {
        self.resources.as_ref()
    }
}

impl From<crate::scene_edit::EditFailure> for SpaceConversionFailure {
    fn from(failure: crate::scene_edit::EditFailure) -> Self {
        use crate::scene_edit::EditCause;
        let cause = match failure.cause {
            EditCause::Runtime { source } => SpaceConversionCause::Runtime { source },
            EditCause::Control { cause } => SpaceConversionCause::Control { cause },
            EditCause::FinalControl => SpaceConversionCause::FinalControl,
            EditCause::CoreRejected => SpaceConversionCause::CoreRejected,
            EditCause::SpaceIssue { issue } => SpaceConversionCause::Issue { issue },
            EditCause::CentralValidation => SpaceConversionCause::CentralValidation,
            EditCause::Publication => SpaceConversionCause::Publication,
            EditCause::Containment { source } => SpaceConversionCause::Containment { source },
            EditCause::Preparation { source } => SpaceConversionCause::Preparation { source },
        };
        Self {
            cause,
            resources: failure.resources,
            _owner: failure.owner,
        }
    }
}

impl fmt::Debug for SpaceConversionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpaceConversionFailure")
            .field("cause", &self.cause)
            .field("resources", &self.resources)
            .finish_non_exhaustive()
    }
}
impl fmt::Display for SpaceConversionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(formatter)
    }
}
impl std::error::Error for SpaceConversionFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}
