use baozi_import::{
    ImportFailure, ImportInitializationError, ImportRunError, RegistryBuildError,
    RegistryRegistrationError, RequestError,
};
use baozi_io::{ImmutableMemoryAssetBuildError, MemoryAssetInsertError, PathError};
use thiserror::Error;

/// A facade-owned failure that preserves the typed lower-layer cause.
#[derive(Debug, Error)]
#[non_exhaustive]
#[allow(
    clippy::large_enum_variant,
    reason = "facade import failures retain complete bounded terminal evidence without failure-path allocation"
)]
pub enum Error {
    #[error(transparent)]
    RegistryRegistration(#[from] RegistryRegistrationError),

    #[error(transparent)]
    RegistryBuild(#[from] RegistryBuildError),

    #[error(transparent)]
    Request(#[from] RequestError),

    #[error(transparent)]
    Path(#[from] PathError),

    #[error(transparent)]
    MemoryAssetInsert(#[from] MemoryAssetInsertError),

    #[error(transparent)]
    ImmutableMemoryAssetBuild(#[from] ImmutableMemoryAssetBuildError),

    #[error("import operation initialization failed: {0}")]
    ImportInitialization(#[source] ImportInitializationError),

    #[error(transparent)]
    Import(#[from] ImportFailure),

    #[error("import terminal finalization failed: {code}")]
    ImportFinalization { code: &'static str },

    #[error(transparent)]
    ImportRun(ImportRunError),
}

impl From<ImportRunError> for Error {
    fn from(error: ImportRunError) -> Self {
        match error {
            ImportRunError::Initialization { source } => Self::ImportInitialization(source),
            ImportRunError::Operation { failure } => Self::Import(failure),
            ImportRunError::Finalization { code } => Self::ImportFinalization { code },
            other => Self::ImportRun(other),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_finalization_survives_the_facade_conversion() {
        let error = Error::from(ImportRunError::Finalization {
            code: "test.finalization",
        });

        assert!(matches!(
            error,
            Error::ImportFinalization {
                code: "test.finalization"
            }
        ));
    }
}
