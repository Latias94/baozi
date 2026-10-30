use std::error::Error;
use std::fmt;

use baozi_core::ValidatedScene;
use baozi_operation::{CancellationOrigin, ControlCause};
use thiserror::Error;

/// Identifies the mechanism that requested post-process cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostProcessCancellationOrigin {
    /// The configured cancellation token was triggered.
    Signal,
    /// A progress observer requested cancellation.
    Observer,
}

/// A post-process-owned projection of cooperative control failure.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PostProcessControlCause {
    /// Cooperative cancellation was observed.
    #[error("post-process operation was cancelled")]
    Cancelled {
        /// The mechanism that requested cancellation.
        origin: PostProcessCancellationOrigin,
    },

    /// The configured deadline elapsed.
    #[error("post-process operation deadline elapsed")]
    DeadlineExceeded,

    /// A host-provided control extension failed while being polled.
    #[error("post-process control extension failed")]
    ExtensionFailed,

    /// Progress reporting violated its monotonic contract.
    #[error("post-process progress contract was violated")]
    ProgressContract,

    /// Bounded progress evidence could not be retained.
    #[error("post-process progress evidence could not be retained")]
    EvidenceUnavailable,

    /// The shared runtime reported an unrecognized fail-closed control cause.
    #[error("post-process control failed closed")]
    RuntimeContract,
}

/// A typed failure from a direct post-process invocation.
#[derive(Debug, Error)]
#[non_exhaustive]
#[allow(
    clippy::large_enum_variant,
    reason = "the failure owns terminal evidence inline without allocating on the rejection path"
)]
pub enum PostProcessRunError {
    /// The bounded resource policy could not be prepared before operation startup.
    #[error("post-process resource policy could not be initialized")]
    ResourcePolicyInitialization,

    /// Triangulation failed without replacing the input Scene.
    #[error("triangulation failed: {failure}")]
    Triangulation {
        #[source]
        failure: crate::TriangulationFailure,
    },
}

/// A direct-operation failure that returns the original immutable input Scene.
pub struct PostProcessRunFailure {
    input: ValidatedScene,
    error: PostProcessRunError,
}

impl fmt::Debug for PostProcessRunFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostProcessRunFailure")
            .field("input_generation", &self.input.generation())
            .field(
                "input_canonical_block_count",
                &self.input.canonical_block_count(),
            )
            .field("error", &self.error)
            .finish()
    }
}

impl PostProcessRunFailure {
    pub(crate) const fn new(input: ValidatedScene, error: PostProcessRunError) -> Self {
        Self { input, error }
    }

    /// Returns the unchanged input Scene.
    pub const fn input(&self) -> &ValidatedScene {
        &self.input
    }

    /// Returns the typed resource-policy or triangulation error.
    pub const fn error(&self) -> &PostProcessRunError {
        &self.error
    }

    /// Consumes the failure into the unchanged input and typed error.
    pub fn into_parts(self) -> (ValidatedScene, PostProcessRunError) {
        (self.input, self.error)
    }
}

impl fmt::Display for PostProcessRunFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "post-process invocation failed: {}", self.error)
    }
}

impl Error for PostProcessRunFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}

pub(crate) fn project_control_cause(cause: ControlCause) -> PostProcessControlCause {
    match cause {
        ControlCause::Cancelled { origin } => PostProcessControlCause::Cancelled {
            origin: match origin {
                CancellationOrigin::Signal => PostProcessCancellationOrigin::Signal,
                CancellationOrigin::Observer => PostProcessCancellationOrigin::Observer,
            },
        },
        ControlCause::DeadlineExceeded => PostProcessControlCause::DeadlineExceeded,
        ControlCause::ObserverPanicked | ControlCause::CancellationSignalPanicked => {
            PostProcessControlCause::ExtensionFailed
        }
        ControlCause::InvalidProgress { .. } => PostProcessControlCause::ProgressContract,
        ControlCause::ProgressStorage { .. } => PostProcessControlCause::EvidenceUnavailable,
        _ => PostProcessControlCause::RuntimeContract,
    }
}
