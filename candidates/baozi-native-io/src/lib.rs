#![forbid(unsafe_code)]

//! Unregistered native filesystem and repository capture candidates for Baozi.
//!
//! The adapter implements the public `baozi_io` contracts using retained native
//! directory authority. Consult [`NATIVE_FS_SUPPORT_STATUS`] before making support
//! claims. The optional `repository-capture` feature adds strict closure capture.

#[cfg(any(
    all(
        target_arch = "x86_64",
        target_vendor = "unknown",
        target_os = "linux",
        target_env = "gnu",
        target_abi = ""
    ),
    all(
        target_arch = "x86_64",
        target_vendor = "pc",
        target_os = "windows",
        target_env = "msvc",
        target_abi = ""
    ),
    all(
        target_arch = "aarch64",
        target_vendor = "apple",
        target_os = "macos",
        target_env = "",
        target_abi = ""
    )
))]
mod fs;
#[cfg(all(
    feature = "repository-capture",
    any(
        all(
            target_arch = "x86_64",
            target_vendor = "unknown",
            target_os = "linux",
            target_env = "gnu",
            target_abi = ""
        ),
        all(
            target_arch = "x86_64",
            target_vendor = "pc",
            target_os = "windows",
            target_env = "msvc",
            target_abi = ""
        ),
        all(
            target_arch = "aarch64",
            target_vendor = "apple",
            target_os = "macos",
            target_env = "",
            target_abi = ""
        )
    )
))]
mod repository;

#[cfg(not(any(
    all(
        target_arch = "x86_64",
        target_vendor = "unknown",
        target_os = "linux",
        target_env = "gnu",
        target_abi = ""
    ),
    all(
        target_arch = "x86_64",
        target_vendor = "pc",
        target_os = "windows",
        target_env = "msvc",
        target_abi = ""
    ),
    all(
        target_arch = "aarch64",
        target_vendor = "apple",
        target_os = "macos",
        target_env = "",
        target_abi = ""
    )
)))]
compile_error!(
    "baozi-native-io-candidate is available only on exact runtime candidate targets: x86_64-unknown-linux-gnu, x86_64-pc-windows-msvc, aarch64-apple-darwin; use memory or custom IO"
);

/// Release support status of Baozi's native filesystem adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeFsSupportStatus {
    /// Candidate code exists, but no target has completed the no-skip conformance gate.
    CandidateUnregistered,
}

/// Native filesystem support is not registered for release use on any target.
pub const NATIVE_FS_SUPPORT_STATUS: NativeFsSupportStatus =
    NativeFsSupportStatus::CandidateUnregistered;

/// Targets with a release-registered, conforming `native-fs` capability.
pub const NATIVE_FS_REGISTERED_TARGETS: &[&str] = &[];

/// Exact targets whose candidate implementations run in platform-local CI.
pub const NATIVE_FS_CANDIDATE_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
];

#[cfg(any(
    all(
        target_arch = "x86_64",
        target_vendor = "unknown",
        target_os = "linux",
        target_env = "gnu",
        target_abi = ""
    ),
    all(
        target_arch = "x86_64",
        target_vendor = "pc",
        target_os = "windows",
        target_env = "msvc",
        target_abi = ""
    ),
    all(
        target_arch = "aarch64",
        target_vendor = "apple",
        target_os = "macos",
        target_env = "",
        target_abi = ""
    )
))]
pub use fs::{FileSystemAssetIo, NativeDirectoryCapability};
#[cfg(all(
    feature = "repository-capture",
    any(
        all(
            target_arch = "x86_64",
            target_vendor = "unknown",
            target_os = "linux",
            target_env = "gnu",
            target_abi = ""
        ),
        all(
            target_arch = "x86_64",
            target_vendor = "pc",
            target_os = "windows",
            target_env = "msvc",
            target_abi = ""
        ),
        all(
            target_arch = "aarch64",
            target_vendor = "apple",
            target_os = "macos",
            target_env = "",
            target_abi = ""
        )
    )
))]
pub use repository::{
    RepositoryCapture, RepositoryCaptureError, RepositoryCaptureErrorKind, RepositoryCaptureStage,
    RepositoryFile, RepositoryLimit, RepositoryLimitSpec, RepositoryLimits, RepositoryLimitsError,
    RepositoryObjectKind, RepositoryPath, RepositoryPathError, RepositoryPathErrorCode,
    RepositoryRoot, RepositoryRootSpec, RepositoryUsage, RepositoryVerifyError,
};

#[cfg(test)]
mod native_fs_status_tests {
    use super::*;

    #[test]
    fn candidate_targets_are_distinct_from_empty_release_registration() {
        assert_eq!(
            NATIVE_FS_SUPPORT_STATUS,
            NativeFsSupportStatus::CandidateUnregistered
        );
        assert!(NATIVE_FS_REGISTERED_TARGETS.is_empty());
        assert_eq!(
            NATIVE_FS_CANDIDATE_TARGETS,
            &[
                "x86_64-unknown-linux-gnu",
                "x86_64-pc-windows-msvc",
                "aarch64-apple-darwin",
            ]
        );
        assert!(
            NATIVE_FS_CANDIDATE_TARGETS
                .iter()
                .all(|candidate| !NATIVE_FS_REGISTERED_TARGETS.contains(candidate))
        );
    }
}
