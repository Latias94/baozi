use crate::CompiledPostProcessPlan;
use crate::operation::{
    PostProcessDimension, PostProcessResourceEvidence, RuntimePostProcessLedger,
};
use baozi_core::{SceneGenerationId, ValidatedScene};
use baozi_operation::{CancellationOrigin, ControlCause, LedgerCause, LedgerFinishError};
use std::error::Error;
use std::fmt;
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

/// A post-process-owned projection of terminal ledger failure.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PostProcessFinalizationError {
    /// Retained storage or reservation capabilities remained live.
    #[error("post-process resources remained live during finalization")]
    OutstandingResources,

    /// A linear work permit remained unresolved.
    #[error("post-process work remained outstanding during finalization")]
    OutstandingWork,

    /// A publication transaction remained in an invalid terminal state.
    #[error("post-process publication state prevented finalization")]
    PublicationState,

    /// Bounded terminal evidence could not be retained.
    #[error("post-process terminal evidence could not be retained")]
    EvidenceUnavailable,

    /// The private ledger shape or accounting contract was inconsistent.
    #[error("post-process accounting contract prevented finalization")]
    AccountingContract,
}

/// A typed failure during compilation or execution of a requested run.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PostProcessError {
    /// Cooperative operation control stopped the invocation.
    #[error("post-process operation stopped: {cause}")]
    Control {
        /// The post-process-owned control cause.
        #[source]
        cause: PostProcessControlCause,
    },

    /// Operation finalization could not produce a clean terminal state.
    #[error("post-process operation finalization failed: {cause}")]
    Finalization {
        /// The post-process-owned finalization cause.
        #[source]
        cause: PostProcessFinalizationError,
    },
}

/// Terminal operation evidence for a failed compilation or execution.
#[derive(Debug, PartialEq, Eq)]
pub struct PostProcessFailure {
    error: PostProcessError,
    resources: PostProcessResourceEvidence,
    cleanup_failure: Option<PostProcessFinalizationError>,
}

impl PostProcessFailure {
    pub(crate) fn new(error: PostProcessError, ledger: RuntimePostProcessLedger) -> Self {
        Self {
            error,
            resources: PostProcessResourceEvidence::from_runtime(ledger),
            cleanup_failure: None,
        }
    }

    pub(crate) fn with_cleanup_failure(
        error: PostProcessError,
        finalization_error: LedgerFinishError<
            crate::operation::PostProcessLedgerPhase,
            PostProcessDimension,
        >,
    ) -> Self {
        let (cause, ledger) = finalization_error.into_parts();
        Self {
            error,
            resources: PostProcessResourceEvidence::from_runtime(ledger),
            cleanup_failure: Some(project_finalization_cause(cause)),
        }
    }

    pub(crate) fn from_finalization_failure(
        finalization_error: LedgerFinishError<
            crate::operation::PostProcessLedgerPhase,
            PostProcessDimension,
        >,
    ) -> Self {
        let (cause, ledger) = finalization_error.into_parts();
        let cause = project_finalization_cause(cause);
        Self {
            error: PostProcessError::Finalization { cause },
            resources: PostProcessResourceEvidence::from_runtime(ledger),
            cleanup_failure: None,
        }
    }

    /// Returns the typed domain error.
    pub const fn error(&self) -> &PostProcessError {
        &self.error
    }

    /// Returns bounded resource evidence without exposing ledger keys or phases.
    pub const fn resources(&self) -> &PostProcessResourceEvidence {
        &self.resources
    }

    /// Returns a later cleanup failure when finalization also failed after the primary error.
    pub const fn cleanup_failure(&self) -> Option<PostProcessFinalizationError> {
        self.cleanup_failure
    }

    /// Consumes the failure into its error, resource evidence, and optional cleanup failure.
    pub fn into_parts(
        self,
    ) -> (
        PostProcessError,
        PostProcessResourceEvidence,
        Option<PostProcessFinalizationError>,
    ) {
        (self.error, self.resources, self.cleanup_failure)
    }
}

impl fmt::Display for PostProcessFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl Error for PostProcessFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}

/// A typed failure before a post-process coordinator exists.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PostProcessInitializationError {
    /// The internal bounded resource policy could not be frozen.
    #[error("post-process resource policy could not be initialized")]
    ResourcePolicy,

    /// The operation coordinator could not retain its complete private vocabulary.
    #[error("post-process operation coordinator could not be initialized")]
    Coordinator,
}

/// Initialization or metered compilation failure before a compiled run exists.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum PostProcessRunError {
    /// The post-process operation could not be initialized.
    #[error("post-process operation initialization failed: {source}")]
    Initialization {
        /// The post-process-owned initialization cause.
        #[source]
        source: PostProcessInitializationError,
    },

    /// The initialized operation failed and retained terminal evidence.
    #[error("post-process operation failed: {failure}")]
    Operation {
        /// Complete terminal evidence for the failed operation.
        #[source]
        failure: PostProcessFailure,
    },
}

impl PostProcessRunError {
    pub(crate) const fn resource_policy_initialization() -> Self {
        Self::Initialization {
            source: PostProcessInitializationError::ResourcePolicy,
        }
    }

    pub(crate) const fn coordinator_initialization() -> Self {
        Self::Initialization {
            source: PostProcessInitializationError::Coordinator,
        }
    }

    pub(crate) const fn operation(failure: PostProcessFailure) -> Self {
        Self::Operation { failure }
    }

    /// Extracts operation evidence without discarding an initialization error.
    pub fn into_operation_failure(
        self,
    ) -> Result<PostProcessFailure, PostProcessInitializationError> {
        match self {
            Self::Operation { failure } => Ok(failure),
            Self::Initialization { source } => Err(source),
        }
    }
}

/// The successful disposition of one requested post-process run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PostProcessRunDisposition {
    /// Requests were evaluated but required no canonical write.
    Unchanged,
}

/// One compiled-step outcome retained by a post-process report.
///
/// The empty-only U7 slice produces no values. Future operations extend this
/// opaque type together with their descriptors and report schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostProcessStepOutcome {
    _private: (),
}

/// One bounded post-process diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostProcessDiagnostic {
    _private: (),
}

/// One structured post-process loss record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostProcessLoss {
    _private: (),
}

/// Immutable evidence for one requested invocation.
#[derive(Debug, PartialEq, Eq)]
pub struct PostProcessReport {
    disposition: PostProcessRunDisposition,
    plan: CompiledPostProcessPlan,
    resources: PostProcessResourceEvidence,
    step_outcomes: Vec<PostProcessStepOutcome>,
    diagnostics: Vec<PostProcessDiagnostic>,
    losses: Vec<PostProcessLoss>,
}

impl PostProcessReport {
    pub(crate) fn unchanged(
        plan: CompiledPostProcessPlan,
        ledger: RuntimePostProcessLedger,
    ) -> Self {
        Self {
            disposition: PostProcessRunDisposition::Unchanged,
            plan,
            resources: PostProcessResourceEvidence::from_runtime(ledger),
            step_outcomes: Vec::new(),
            diagnostics: Vec::new(),
            losses: Vec::new(),
        }
    }

    /// Returns the successful disposition.
    pub const fn disposition(&self) -> PostProcessRunDisposition {
        self.disposition
    }

    /// Returns the exact plan compiled against the returned Scene.
    pub const fn plan(&self) -> &CompiledPostProcessPlan {
        &self.plan
    }

    /// Returns the generation preserved by this unchanged invocation.
    pub const fn preserved_generation(&self) -> SceneGenerationId {
        self.plan.input_generation()
    }

    /// Returns whether the report is generation-bound to the provided Scene.
    pub fn matches_scene(&self, scene: &ValidatedScene) -> bool {
        self.preserved_generation() == scene.generation()
    }

    /// Returns bounded resource evidence without exposing ledger keys or phases.
    pub const fn resources(&self) -> &PostProcessResourceEvidence {
        &self.resources
    }

    /// Returns source-ordered outcomes for every compiled step.
    pub fn step_outcomes(&self) -> &[PostProcessStepOutcome] {
        &self.step_outcomes
    }

    /// Returns bounded post-process diagnostics.
    pub fn diagnostics(&self) -> &[PostProcessDiagnostic] {
        &self.diagnostics
    }

    /// Returns structured post-process loss evidence.
    pub fn losses(&self) -> &[PostProcessLoss] {
        &self.losses
    }
}

/// The complete successful result of one requested invocation.
#[must_use = "the post-process success carries the preserved or newly published Scene"]
pub struct PostProcessRunOutcome {
    scene: ValidatedScene,
    report: PostProcessReport,
}

impl PostProcessRunOutcome {
    pub(crate) const fn new(scene: ValidatedScene, report: PostProcessReport) -> Self {
        Self { scene, report }
    }

    /// Returns the successful disposition.
    pub const fn disposition(&self) -> PostProcessRunDisposition {
        self.report.disposition()
    }

    /// Returns the preserved input or newly published Scene.
    pub const fn scene(&self) -> &ValidatedScene {
        &self.scene
    }

    /// Returns immutable invocation evidence.
    pub const fn report(&self) -> &PostProcessReport {
        &self.report
    }

    /// Consumes the success into the Scene owner and invocation report.
    pub fn into_parts(self) -> (ValidatedScene, PostProcessReport) {
        (self.scene, self.report)
    }
}

impl fmt::Debug for PostProcessRunOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostProcessRunOutcome")
            .field("scene_generation", &self.scene.generation())
            .field(
                "scene_canonical_block_count",
                &self.scene.canonical_block_count(),
            )
            .field("report", &self.report)
            .finish()
    }
}

/// A compile failure that returns the unchanged input Scene.
pub struct PostProcessCompileFailure {
    input: ValidatedScene,
    error: PostProcessRunError,
}

impl fmt::Debug for PostProcessCompileFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostProcessCompileFailure")
            .field("input_generation", &self.input.generation())
            .field(
                "input_canonical_block_count",
                &self.input.canonical_block_count(),
            )
            .field("error", &self.error)
            .finish()
    }
}

impl PostProcessCompileFailure {
    pub(crate) const fn new(input: ValidatedScene, error: PostProcessRunError) -> Self {
        Self { input, error }
    }

    /// Returns the unchanged input Scene.
    pub const fn input(&self) -> &ValidatedScene {
        &self.input
    }

    /// Returns the typed initialization or compilation error.
    pub const fn error(&self) -> &PostProcessRunError {
        &self.error
    }

    /// Consumes the failure into the unchanged input and typed error.
    pub fn into_parts(self) -> (ValidatedScene, PostProcessRunError) {
        (self.input, self.error)
    }
}

impl fmt::Display for PostProcessCompileFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "post-process compilation failed: {}", self.error)
    }
}

impl Error for PostProcessCompileFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}

/// An execution failure that returns the unchanged input Scene.
pub struct PostProcessExecutionFailure {
    input: ValidatedScene,
    failure: PostProcessFailure,
}

impl fmt::Debug for PostProcessExecutionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostProcessExecutionFailure")
            .field("input_generation", &self.input.generation())
            .field(
                "input_canonical_block_count",
                &self.input.canonical_block_count(),
            )
            .field("failure", &self.failure)
            .finish()
    }
}

impl PostProcessExecutionFailure {
    pub(crate) const fn new(input: ValidatedScene, failure: PostProcessFailure) -> Self {
        Self { input, failure }
    }

    /// Returns the unchanged input Scene.
    pub const fn input(&self) -> &ValidatedScene {
        &self.input
    }

    /// Returns terminal operation evidence for the failed execution.
    pub const fn failure(&self) -> &PostProcessFailure {
        &self.failure
    }

    /// Consumes the failure into the unchanged input and terminal evidence.
    pub fn into_parts(self) -> (ValidatedScene, PostProcessFailure) {
        (self.input, self.failure)
    }
}

impl fmt::Display for PostProcessExecutionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "post-process execution failed: {}", self.failure)
    }
}

impl Error for PostProcessExecutionFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.failure)
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

fn project_finalization_cause(
    cause: LedgerCause<PostProcessDimension>,
) -> PostProcessFinalizationError {
    match cause {
        LedgerCause::LiveReservationsRemain { .. }
        | LedgerCause::ActiveReservationsRemain { .. } => {
            PostProcessFinalizationError::OutstandingResources
        }
        LedgerCause::OutstandingLinearWorkPermits { .. } => {
            PostProcessFinalizationError::OutstandingWork
        }
        LedgerCause::PendingPublicationsRemain
        | LedgerCause::PublicationAlreadyPending
        | LedgerCause::PublicationAlreadyCompleted
        | LedgerCause::InactivePublication
        | LedgerCause::PublicationLiveMismatch { .. } => {
            PostProcessFinalizationError::PublicationState
        }
        LedgerCause::Evidence { .. } => PostProcessFinalizationError::EvidenceUnavailable,
        _ => PostProcessFinalizationError::AccountingContract,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialization_mismatch_keeps_a_postprocess_owned_cause() {
        let run_error = PostProcessRunError::coordinator_initialization();

        let initialization_source = Error::source(&run_error)
            .and_then(|source| source.downcast_ref::<PostProcessInitializationError>())
            .expect("run error must retain the typed initialization source");
        assert_eq!(
            initialization_source,
            &PostProcessInitializationError::Coordinator
        );

        let initialization = run_error
            .into_operation_failure()
            .expect_err("coordinator initialization cannot be an operation failure");
        assert_eq!(initialization, PostProcessInitializationError::Coordinator);
    }
}
