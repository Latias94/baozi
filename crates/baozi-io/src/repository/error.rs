use std::error::Error;
use std::fmt;
use std::io;

use thiserror::Error;

use super::{RepositoryLimit, RepositoryPath, RepositoryPathErrorCode};

/// Capture operation at which repository observation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RepositoryCaptureStage {
    ValidateRoots,
    ResolveComponent,
    EnumerateDirectory,
    OpenDirectory,
    InspectDirectory,
    OpenFile,
    InspectFile,
    ReadFile,
    ReenumerateDirectory,
    Finalize,
}

/// Object kind observed at a repository path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RepositoryObjectKind {
    File,
    Directory,
    SymlinkOrReparsePoint,
    Fifo,
    Socket,
    CharacterDevice,
    BlockDevice,
    Other,
}

/// Typed reason for a repository capture failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RepositoryCaptureErrorKind {
    NoRoots,
    DuplicateRoot,
    OverlappingRoots,
    PortableNameCollision,
    InvalidPath(RepositoryPathErrorCode),
    LimitExceeded {
        limit: RepositoryLimit,
        attempted: u64,
        maximum: u64,
    },
    NonUtf8Name,
    ExactSpellingMismatch,
    NotFound,
    PermissionDenied,
    WouldBlock,
    Unsupported,
    UnsupportedIdentityBackend,
    InvalidInput,
    ExpectedObject {
        expected: RepositoryObjectKind,
        actual: RepositoryObjectKind,
    },
    UnsupportedObject(RepositoryObjectKind),
    IdentityUnavailable,
    IdentityAlias,
    HardLinkedFile {
        links: u64,
    },
    DirectoryEntriesChanged,
    ObjectChangedDuringRead,
    CapacityExceeded,
    Backend,
}

/// Bounded failure emitted while capturing a repository closure.
pub struct RepositoryCaptureError {
    stage: RepositoryCaptureStage,
    kind: RepositoryCaptureErrorKind,
    path: Option<RepositoryPath>,
}

impl RepositoryCaptureError {
    pub(super) fn new(stage: RepositoryCaptureStage, kind: RepositoryCaptureErrorKind) -> Self {
        Self {
            stage,
            kind,
            path: None,
        }
    }

    pub(super) fn at(
        stage: RepositoryCaptureStage,
        kind: RepositoryCaptureErrorKind,
        path: &RepositoryPath,
    ) -> Self {
        Self {
            stage,
            kind,
            path: path.try_clone().ok(),
        }
    }

    pub(super) fn from_io(
        stage: RepositoryCaptureStage,
        error: io::Error,
        path: Option<&RepositoryPath>,
    ) -> Self {
        let kind = match error.kind() {
            io::ErrorKind::NotFound => RepositoryCaptureErrorKind::NotFound,
            io::ErrorKind::PermissionDenied => RepositoryCaptureErrorKind::PermissionDenied,
            io::ErrorKind::WouldBlock => RepositoryCaptureErrorKind::WouldBlock,
            io::ErrorKind::Unsupported => RepositoryCaptureErrorKind::Unsupported,
            io::ErrorKind::InvalidInput
            | io::ErrorKind::InvalidData
            | io::ErrorKind::NotADirectory => RepositoryCaptureErrorKind::InvalidInput,
            _ => RepositoryCaptureErrorKind::Backend,
        };
        Self {
            stage,
            kind,
            path: path.and_then(|path| path.try_clone().ok()),
        }
    }

    pub const fn stage(&self) -> RepositoryCaptureStage {
        self.stage
    }

    pub const fn kind(&self) -> RepositoryCaptureErrorKind {
        self.kind
    }

    pub fn path(&self) -> Option<&RepositoryPath> {
        self.path.as_ref()
    }
}

impl fmt::Debug for RepositoryCaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RepositoryCaptureError")
            .field("stage", &self.stage)
            .field("kind", &self.kind)
            .field("path", &self.path)
            .finish()
    }
}

impl fmt::Display for RepositoryCaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        ::core::write!(
            formatter,
            "repository capture failed during {:?}: {:?}",
            self.stage,
            self.kind
        )
    }
}

impl Error for RepositoryCaptureError {}

/// Difference between a baseline capture and a complete endpoint recapture.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RepositoryVerifyError {
    #[error("repository capture belongs to another authority")]
    AuthorityMismatch,

    #[error("repository recapture failed")]
    Recapture(#[source] RepositoryCaptureError),

    #[error("verification path storage allocation failed")]
    AllocationFailed,

    #[error("repository path was added")]
    Added { path: RepositoryPath },

    #[error("repository path was removed")]
    Removed { path: RepositoryPath },

    #[error("repository path changed object kind")]
    KindChanged {
        path: RepositoryPath,
        expected: RepositoryObjectKind,
        actual: RepositoryObjectKind,
    },

    #[error("repository path was replaced")]
    Replaced { path: RepositoryPath },

    #[error("repository file content changed")]
    ContentChanged { path: RepositoryPath },
}
