use baozi_operation::__retained_store_integration::authority::{
    FiniteStoreExitCause, FiniteStoreExitError,
};
use baozi_operation::{
    CancellationToken, ControlCause, OperationControl, OperationOwner, ProgressPhase, ProgressUnit,
    StoreKeyDomain,
};
use std::fmt;
use std::time::Instant;

mod resource;
pub(crate) mod runtime;

pub use resource::PostProcessResourceEvidence;

/// Owns the original finite invocation error and any retained failure residue.
///
/// Its diagnostics borrow the original error, including after the invocation's host
/// and source have been released. No separate terminal resource snapshot is required.
/// The original owner can move between threads. It is not shared between threads:
/// retained invocation state includes local cells and Send-only control extensions.
#[derive(Debug)]
pub struct PostProcessInvocationFailure {
    inner:
        FiniteStoreExitError<PostProcessOwner, PostProcessProgressPhase, PostProcessProgressUnit>,
}

impl PostProcessInvocationFailure {
    pub(crate) fn new(
        inner: FiniteStoreExitError<
            PostProcessOwner,
            PostProcessProgressPhase,
            PostProcessProgressUnit,
        >,
    ) -> Self {
        Self { inner }
    }

    /// Borrows the original invocation's startup, body, or finalization cause.
    pub const fn cause(&self) -> &FiniteStoreExitCause {
        self.inner.cause()
    }

    /// Borrows the original domain, including when startup produced no qualified facts.
    pub fn domain(&self) -> Option<&StoreKeyDomain> {
        self.inner.domain()
    }

    /// Borrows the detailed control cause observed by the original finite freeze, if any.
    pub const fn original_freeze_control(&self) -> Option<&ControlCause> {
        self.inner.original_freeze_control()
    }

    /// Returns whether the original error owns a root or low-level failure residue.
    pub fn has_residue(&self) -> bool {
        self.inner.has_residue()
    }
}

impl fmt::Display for PostProcessInvocationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, formatter)
    }
}

impl std::error::Error for PostProcessInvocationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.inner)
    }
}

/// Owner marker for direct post-process operation capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PostProcessOwner;

impl OperationOwner for PostProcessOwner {}

/// Progress phases for direct post-process operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum PostProcessProgressPhase {
    Validation,
    Publication,
}

impl ProgressPhase for PostProcessProgressPhase {
    fn order(self) -> u32 {
        self as u32
    }

    fn stable_id(self) -> &'static str {
        match self {
            Self::Validation => "postprocess-validation",
            Self::Publication => "postprocess-publication",
        }
    }
}

/// Stable post-process progress units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum PostProcessProgressUnit {
    WorkItems,
    Scene,
}

impl ProgressUnit for PostProcessProgressUnit {
    fn stable_id(self) -> &'static str {
        match self {
            Self::WorkItems => "work_items",
            Self::Scene => "scene",
        }
    }
}

pub(crate) type RuntimePostProcessControl =
    OperationControl<PostProcessProgressPhase, PostProcessProgressUnit>;

/// A cloneable cancellation token owned by the post-process API.
#[derive(Debug, Clone, Default)]
pub struct PostProcessCancellationToken {
    inner: CancellationToken,
}

impl PostProcessCancellationToken {
    /// Creates an active token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation. This transition cannot be reversed.
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    /// Returns whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }
}

/// Invocation-local cancellation and deadline control.
#[derive(Default)]
pub struct PostProcessControl {
    inner: RuntimePostProcessControl,
}

impl PostProcessControl {
    /// Installs a post-process-owned cancellation token.
    pub fn with_cancellation(mut self, cancellation: PostProcessCancellationToken) -> Self {
        self.inner = self.inner.with_cancellation(cancellation.inner);
        self
    }

    /// Installs a deadline backstop.
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.inner = self.inner.with_deadline(deadline);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_cancellation_signal(
        mut self,
        cancellation: impl baozi_operation::CancellationSignal,
    ) -> Self {
        self.inner = self.inner.with_cancellation(cancellation);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_progress_observer(
        mut self,
        observer: impl baozi_operation::ProgressObserver<
            PostProcessProgressPhase,
            PostProcessProgressUnit,
        >,
    ) -> Self {
        self.inner = self.inner.with_progress_observer(observer);
        self
    }

    pub(crate) fn into_runtime(self) -> RuntimePostProcessControl {
        self.inner
    }
}

impl fmt::Debug for PostProcessControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostProcessControl")
            .finish_non_exhaustive()
    }
}
