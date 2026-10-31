//! Strict, handle-relative repository closure capture.

mod error;
mod limits;
mod native;
mod path;
mod walk;

pub use error::{
    RepositoryCaptureError, RepositoryCaptureErrorKind, RepositoryCaptureStage,
    RepositoryObjectKind, RepositoryVerifyError,
};
pub use limits::{RepositoryLimit, RepositoryLimitSpec, RepositoryLimits, RepositoryLimitsError};
pub use native::{
    RepositoryCapture, RepositoryFile, RepositoryRoot, RepositoryRootSpec, RepositoryUsage,
};
pub use path::{RepositoryPath, RepositoryPathError, RepositoryPathErrorCode};

#[cfg(test)]
mod tests;
