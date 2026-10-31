use std::fmt;

use baozi_core::__scene_integration::integration::SceneEditEvidenceTag;
use baozi_core::{SceneLookupError, TopologyKind};
use baozi_operation::__retained_store_integration::activation::StoreActivationContainment;
use baozi_operation::__retained_store_integration::authority::RetainedStoreStartError;
use baozi_operation::{
    ControlCause, PublishedRichFailureStore, PublishedStoreFallbackOutput, RecordBlock,
    StorePrepareError, StoreRuntimeError,
};
use thiserror::Error;

use crate::error::project_control_cause;
use crate::{PostProcessControlCause, PostProcessResourceEvidence, TriangulationIssue};

/// Why one direct triangulation could not publish an output.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TriangulationCause {
    #[error("the target primitive could not be looked up: {source}")]
    Target {
        #[source]
        source: SceneLookupError,
    },
    #[error("triangulation does not support {topology:?} topology")]
    UnsupportedTopology { topology: TopologyKind },
    #[error("the validated primitive has no supported vertex POSITION stream")]
    Positions,
    #[error("polygon {face} cannot be triangulated: {issue:?}")]
    Polygon {
        face: u32,
        issue: TriangulationIssue,
    },
    #[error("triangulation size or work bound overflowed")]
    SizeOverflow,
    #[error("triangulation requires {required} work units, above the {maximum} unit limit")]
    WorkLimit { required: u64, maximum: u64 },
    #[error("triangulation stopped: {cause}")]
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
    #[error("triangulation storage or work admission failed: {source}")]
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

impl From<StoreRuntimeError> for TriangulationCause {
    fn from(source: StoreRuntimeError) -> Self {
        match source {
            StoreRuntimeError::Control(cause) => Self::control(cause),
            source => Self::Runtime { source },
        }
    }
}

impl TriangulationCause {
    pub(crate) fn control(cause: ControlCause) -> Self {
        Self::Control {
            cause: project_control_cause(cause),
        }
    }
}

pub(crate) enum FailureOwner {
    None,
    Fallback {
        _owner: PublishedStoreFallbackOutput,
    },
    Rich {
        _owner: PublishedRichFailureStore<RecordBlock, SceneEditEvidenceTag>,
    },
}

/// A typed direct-operation error and the actual published terminal evidence, when available.
///
/// Startup and fatal containment have no fabricated ordinary terminal resource snapshot.
pub struct TriangulationFailure {
    cause: TriangulationCause,
    resources: Option<PostProcessResourceEvidence>,
    _owner: FailureOwner,
}

impl TriangulationFailure {
    pub(crate) const fn new(cause: TriangulationCause) -> Self {
        Self {
            cause,
            resources: None,
            _owner: FailureOwner::None,
        }
    }

    pub(crate) fn terminal(
        cause: TriangulationCause,
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
    pub const fn cause(&self) -> &TriangulationCause {
        &self.cause
    }

    /// Returns real terminal resource evidence; absent for startup failure and fatal containment.
    pub const fn resources(&self) -> Option<&PostProcessResourceEvidence> {
        self.resources.as_ref()
    }
}

impl fmt::Debug for TriangulationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TriangulationFailure")
            .field("cause", &self.cause)
            .field("resources", &self.resources)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for TriangulationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(formatter)
    }
}

impl std::error::Error for TriangulationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

impl From<crate::scene_edit::EditFailure> for TriangulationFailure {
    fn from(failure: crate::scene_edit::EditFailure) -> Self {
        use crate::scene_edit::EditCause;
        let cause = match failure.cause {
            EditCause::Runtime { source } => TriangulationCause::Runtime { source },
            EditCause::Control { cause } => TriangulationCause::Control { cause },
            EditCause::FinalControl => TriangulationCause::FinalControl,
            EditCause::CoreRejected => TriangulationCause::CoreRejected,
            EditCause::SpaceIssue { .. } => TriangulationCause::CoreRejected,
            EditCause::CentralValidation => TriangulationCause::CentralValidation,
            EditCause::Publication => TriangulationCause::Publication,
            EditCause::Containment { source } => TriangulationCause::Containment { source },
            EditCause::Preparation { source } => TriangulationCause::Preparation { source },
        };
        Self {
            cause,
            resources: failure.resources,
            _owner: failure.owner,
        }
    }
}
