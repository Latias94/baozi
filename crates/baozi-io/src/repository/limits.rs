use thiserror::Error;

use super::RepositoryPath;

/// A separately accounted repository capture resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RepositoryLimit {
    Roots,
    Entries,
    Files,
    Directories,
    Depth,
    PathBytes,
    FileBytes,
    TotalBytes,
    Work,
}

/// Unvalidated caller policy for one repository capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepositoryLimitSpec {
    pub max_roots: u64,
    pub max_entries: u64,
    pub max_files: u64,
    pub max_directories: u64,
    pub max_depth: u64,
    pub max_path_bytes: u64,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_work: u64,
}

impl RepositoryLimitSpec {
    pub fn validate(self) -> Result<RepositoryLimits, RepositoryLimitsError> {
        for (limit, value) in [
            (RepositoryLimit::Roots, self.max_roots),
            (RepositoryLimit::Entries, self.max_entries),
            (RepositoryLimit::Files, self.max_files),
            (RepositoryLimit::Directories, self.max_directories),
            (RepositoryLimit::Depth, self.max_depth),
            (RepositoryLimit::PathBytes, self.max_path_bytes),
            (RepositoryLimit::FileBytes, self.max_file_bytes),
            (RepositoryLimit::TotalBytes, self.max_total_bytes),
            (RepositoryLimit::Work, self.max_work),
        ] {
            if value == 0 {
                return Err(RepositoryLimitsError::Zero { limit });
            }
        }

        let hard_path_maximum = u64::try_from(RepositoryPath::HARD_MAX_BYTES)
            .map_err(|_| RepositoryLimitsError::ArithmeticOverflow)?;
        if self.max_path_bytes > hard_path_maximum {
            return Err(RepositoryLimitsError::PathExceedsHardMaximum {
                actual: self.max_path_bytes,
                hard_maximum: hard_path_maximum,
            });
        }

        self.max_files
            .checked_add(self.max_directories)
            .ok_or(RepositoryLimitsError::ArithmeticOverflow)?;

        for (limit, value) in [
            (RepositoryLimit::Roots, self.max_roots),
            (RepositoryLimit::Entries, self.max_entries),
            (RepositoryLimit::Files, self.max_files),
            (RepositoryLimit::Directories, self.max_directories),
            (RepositoryLimit::Depth, self.max_depth),
            (RepositoryLimit::FileBytes, self.max_file_bytes),
            (RepositoryLimit::TotalBytes, self.max_total_bytes),
        ] {
            usize::try_from(value)
                .map_err(|_| RepositoryLimitsError::PlatformCapacity { limit, value })?;
        }

        Ok(RepositoryLimits {
            max_roots: self.max_roots,
            max_entries: self.max_entries,
            max_files: self.max_files,
            max_directories: self.max_directories,
            max_depth: self.max_depth,
            max_path_bytes: self.max_path_bytes,
            max_file_bytes: self.max_file_bytes,
            max_total_bytes: self.max_total_bytes,
            max_work: self.max_work,
        })
    }
}

/// Failure to validate a finite repository limit specification.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RepositoryLimitsError {
    #[error("{limit:?} repository limit must be nonzero")]
    Zero { limit: RepositoryLimit },

    #[error("repository path limit {actual} exceeds the hard maximum {hard_maximum}")]
    PathExceedsHardMaximum { actual: u64, hard_maximum: u64 },

    #[error("{limit:?} repository limit {value} exceeds platform capacity")]
    PlatformCapacity { limit: RepositoryLimit, value: u64 },

    #[error("repository limit arithmetic overflowed")]
    ArithmeticOverflow,
}

/// Validated finite limits for one repository capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepositoryLimits {
    max_roots: u64,
    max_entries: u64,
    max_files: u64,
    max_directories: u64,
    max_depth: u64,
    max_path_bytes: u64,
    max_file_bytes: u64,
    max_total_bytes: u64,
    max_work: u64,
}

impl RepositoryLimits {
    pub const fn max_roots(self) -> u64 {
        self.max_roots
    }

    pub const fn max_entries(self) -> u64 {
        self.max_entries
    }

    pub const fn max_files(self) -> u64 {
        self.max_files
    }

    pub const fn max_directories(self) -> u64 {
        self.max_directories
    }

    pub const fn max_depth(self) -> u64 {
        self.max_depth
    }

    pub const fn max_path_bytes(self) -> u64 {
        self.max_path_bytes
    }

    pub const fn max_file_bytes(self) -> u64 {
        self.max_file_bytes
    }

    pub const fn max_total_bytes(self) -> u64 {
        self.max_total_bytes
    }

    pub const fn max_work(self) -> u64 {
        self.max_work
    }
}
