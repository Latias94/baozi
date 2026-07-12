use std::error::Error;
use std::fmt;
use std::io;
use std::sync::Arc;

use thiserror::Error;

type SharedError = Arc<dyn Error + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathErrorCode {
    Empty,
    TooLong,
    Absolute,
    Namespace,
    DriveQualified,
    AlternateDataStream,
    ControlCharacter,
    EmptyComponent,
    Escape,
    InvalidCharacter,
    ShortNameAlias,
    TrailingDotOrSpace,
    ReservedDevice,
    AllocationFailed,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PathError {
    #[error("asset path is empty")]
    Empty,

    #[error("asset path is {actual} bytes; the hard maximum is {maximum}")]
    TooLong { actual: usize, maximum: usize },

    #[error("absolute asset paths are not allowed")]
    Absolute,

    #[error("Win32 and NT namespace paths are not allowed")]
    Namespace,

    #[error("drive-qualified asset paths are not allowed")]
    DriveQualified,

    #[error("alternate data stream syntax is not allowed in component {component}")]
    AlternateDataStream { component: usize },

    #[error("control character at byte {byte_index} is not allowed in an asset path")]
    ControlCharacter { byte_index: usize },

    #[error("empty asset path component {component} is not allowed")]
    EmptyComponent { component: usize },

    #[error("asset path escapes its logical base")]
    Escape,

    #[error("platform-ambiguous character is not allowed in component {component}")]
    InvalidCharacter { component: usize },

    #[error("Windows short-name alias syntax is not allowed in component {component}")]
    ShortNameAlias { component: usize },

    #[error("component {component} ends in a dot or space")]
    TrailingDotOrSpace { component: usize },

    #[error("component {component} is a reserved Windows device alias")]
    ReservedDevice { component: usize },

    #[error("asset path storage allocation failed")]
    AllocationFailed,
}

impl PathError {
    pub const fn code(&self) -> PathErrorCode {
        match self {
            Self::Empty => PathErrorCode::Empty,
            Self::TooLong { .. } => PathErrorCode::TooLong,
            Self::Absolute => PathErrorCode::Absolute,
            Self::Namespace => PathErrorCode::Namespace,
            Self::DriveQualified => PathErrorCode::DriveQualified,
            Self::AlternateDataStream { .. } => PathErrorCode::AlternateDataStream,
            Self::ControlCharacter { .. } => PathErrorCode::ControlCharacter,
            Self::EmptyComponent { .. } => PathErrorCode::EmptyComponent,
            Self::Escape => PathErrorCode::Escape,
            Self::InvalidCharacter { .. } => PathErrorCode::InvalidCharacter,
            Self::ShortNameAlias { .. } => PathErrorCode::ShortNameAlias,
            Self::TrailingDotOrSpace { .. } => PathErrorCode::TrailingDotOrSpace,
            Self::ReservedDevice { .. } => PathErrorCode::ReservedDevice,
            Self::AllocationFailed => PathErrorCode::AllocationFailed,
        }
    }
}

/// Failure to add a payload to a [`crate::MemoryAssetIo`] namespace.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MemoryAssetInsertError {
    #[error("memory asset index allocation failed")]
    AllocationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetOpenStage {
    RootCapability,
    Resolve,
    Open,
    InspectObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetObjectKind {
    Directory,
    SymlinkOrReparsePoint,
    Fifo,
    Socket,
    CharacterDevice,
    BlockDevice,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetOpenErrorKind {
    NotFound,
    PermissionDenied,
    CapabilityUnavailable,
    ContainmentUnproven,
    InvalidInput,
    WouldBlock,
    RootNotDirectory,
    NonRegularFile(AssetObjectKind),
    CapacityExceeded,
    Backend,
}

#[derive(Clone)]
pub struct AssetOpenError {
    stage: AssetOpenStage,
    kind: AssetOpenErrorKind,
    source: Option<SharedError>,
}

impl AssetOpenError {
    pub const fn new(stage: AssetOpenStage, kind: AssetOpenErrorKind) -> Self {
        Self {
            stage,
            kind,
            source: None,
        }
    }

    pub fn from_io(stage: AssetOpenStage, error: io::Error) -> Self {
        let kind = match (stage, error.kind()) {
            (AssetOpenStage::RootCapability, io::ErrorKind::NotADirectory) => {
                AssetOpenErrorKind::RootNotDirectory
            }
            (_, io::ErrorKind::IsADirectory) => {
                AssetOpenErrorKind::NonRegularFile(AssetObjectKind::Directory)
            }
            (_, io::ErrorKind::NotFound) => AssetOpenErrorKind::NotFound,
            (_, io::ErrorKind::PermissionDenied) => AssetOpenErrorKind::PermissionDenied,
            (_, io::ErrorKind::Unsupported) => AssetOpenErrorKind::CapabilityUnavailable,
            (
                _,
                io::ErrorKind::InvalidInput
                | io::ErrorKind::InvalidData
                | io::ErrorKind::NotADirectory,
            ) => AssetOpenErrorKind::InvalidInput,
            (_, io::ErrorKind::WouldBlock) => AssetOpenErrorKind::WouldBlock,
            _ => AssetOpenErrorKind::Backend,
        };
        Self {
            stage,
            kind,
            source: Some(Arc::new(error)),
        }
    }

    pub fn backend(stage: AssetOpenStage, error: impl Error + Send + Sync + 'static) -> Self {
        Self {
            stage,
            kind: AssetOpenErrorKind::Backend,
            source: Some(Arc::new(error)),
        }
    }

    pub const fn stage(&self) -> AssetOpenStage {
        self.stage
    }

    pub const fn kind(&self) -> AssetOpenErrorKind {
        self.kind
    }

    pub fn source_ref(&self) -> Option<&(dyn Error + Send + Sync + 'static)> {
        self.source.as_deref()
    }
}

impl fmt::Debug for AssetOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AssetOpenError")
            .field("stage", &self.stage)
            .field("kind", &self.kind)
            .field("has_source", &self.source.is_some())
            .finish()
    }
}

impl fmt::Display for AssetOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "asset open failed during {:?}: {:?}",
            self.stage, self.kind
        )
    }
}

impl Error for AssetOpenError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetReadErrorKind {
    Interrupted,
    WouldBlock,
    UnexpectedEof,
    InvalidData,
    ContractViolation(AssetReadContractViolation),
    Backend,
}

/// A trusted `AssetReader` violated the `OpenedAsset` byte-stream contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetReadContractViolation {
    CountExceedsBuffer { returned: usize, capacity: usize },
    ByteCountOverflow { consumed: u64, returned: usize },
    ExactLengthExceeded { expected: u64, observed: u64 },
    ExactLengthTruncated { expected: u64, observed: u64 },
}

#[derive(Clone)]
pub struct AssetReadError {
    kind: AssetReadErrorKind,
    source: Option<SharedError>,
}

impl AssetReadError {
    pub const fn new(kind: AssetReadErrorKind) -> Self {
        Self { kind, source: None }
    }

    pub fn from_io(error: io::Error) -> Self {
        let kind = match error.kind() {
            io::ErrorKind::Interrupted => AssetReadErrorKind::Interrupted,
            io::ErrorKind::WouldBlock => AssetReadErrorKind::WouldBlock,
            io::ErrorKind::UnexpectedEof => AssetReadErrorKind::UnexpectedEof,
            io::ErrorKind::InvalidData => AssetReadErrorKind::InvalidData,
            _ => AssetReadErrorKind::Backend,
        };
        Self {
            kind,
            source: Some(Arc::new(error)),
        }
    }

    pub fn backend(error: impl Error + Send + Sync + 'static) -> Self {
        Self {
            kind: AssetReadErrorKind::Backend,
            source: Some(Arc::new(error)),
        }
    }

    pub const fn contract_violation(violation: AssetReadContractViolation) -> Self {
        Self::new(AssetReadErrorKind::ContractViolation(violation))
    }

    pub const fn kind(&self) -> AssetReadErrorKind {
        self.kind
    }

    pub fn source_ref(&self) -> Option<&(dyn Error + Send + Sync + 'static)> {
        self.source.as_deref()
    }
}

impl fmt::Debug for AssetReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AssetReadError")
            .field("kind", &self.kind)
            .field("has_source", &self.source.is_some())
            .finish()
    }
}

impl fmt::Display for AssetReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "asset read failed: {:?}", self.kind)
    }
}

impl Error for AssetReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_errors_retain_typed_sources() {
        let error = AssetOpenError::from_io(
            AssetOpenStage::Open,
            io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
        );

        assert_eq!(error.stage(), AssetOpenStage::Open);
        assert_eq!(error.kind(), AssetOpenErrorKind::PermissionDenied);
        assert_eq!(
            error
                .source_ref()
                .and_then(|source| source.downcast_ref::<io::Error>())
                .map(io::Error::kind),
            Some(io::ErrorKind::PermissionDenied)
        );
    }

    #[test]
    fn read_errors_retain_typed_sources() {
        let error = AssetReadError::from_io(io::Error::new(
            io::ErrorKind::WouldBlock,
            "temporarily unavailable",
        ));

        assert_eq!(error.kind(), AssetReadErrorKind::WouldBlock);
        assert_eq!(
            error
                .source_ref()
                .and_then(|source| source.downcast_ref::<io::Error>())
                .map(io::Error::kind),
            Some(io::ErrorKind::WouldBlock)
        );
    }

    #[test]
    fn public_error_messages_do_not_copy_unbounded_backend_detail() {
        let detail = "x".repeat(64 * 1024);
        let open = AssetOpenError::backend(AssetOpenStage::Open, io::Error::other(detail.clone()));
        let read = AssetReadError::backend(io::Error::other(detail));

        assert!(open.to_string().len() < 128);
        assert!(read.to_string().len() < 128);
        assert!(open.source().is_some());
        assert!(read.source().is_some());
    }
}
