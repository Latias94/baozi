#![forbid(unsafe_code)]

//! Runtime-neutral, capability-bound asset IO contracts.
//!
//! The native filesystem adapter is available only through the explicitly unregistered
//! `fs-candidate` feature. Consult [`NATIVE_FS_SUPPORT_STATUS`] before making support claims.

mod error;
#[cfg(all(
    feature = "fs-candidate",
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
mod fs;
mod immutable_memory;
mod memory;
mod path;
#[cfg(all(
    feature = "repository-fs-candidate",
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
mod resource;
mod semantic_identity;

#[cfg(all(
    feature = "fs-candidate",
    not(any(
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
    ))
))]
compile_error!(
    "baozi-io/fs-candidate is available only on exact runtime candidate targets: x86_64-unknown-linux-gnu, x86_64-pc-windows-msvc, aarch64-apple-darwin; use memory or custom IO"
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

pub use error::{
    AssetObjectKind, AssetOpenError, AssetOpenErrorKind, AssetOpenStage,
    AssetReadContractViolation, AssetReadError, AssetReadErrorKind, ImmutableMemoryAssetBuildError,
    MemoryAssetInsertError, PathError, PathErrorCode,
};
#[cfg(all(
    feature = "fs-candidate",
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
pub use fs::{FileSystemAssetIo, NativeDirectoryCapability};
pub use immutable_memory::ImmutableMemoryAssetIo;
pub use memory::MemoryAssetIo;
pub use path::AssetPath;
#[cfg(all(
    feature = "repository-fs-candidate",
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
pub use resource::{AssetIo, AssetLength, AssetReader, AuthorityDomain, OpenedAsset};
/// Stable asset-IO semantic registry used by import's deep resource graph module.
///
/// This is intentionally hidden from the ordinary root API. Importers may provide a custom
/// resolver, but only the import session can decide whether a registered profile is admitted into
/// a successful-input manifest.
#[doc(hidden)]
pub mod __semantic_identity {
    pub use crate::semantic_identity::{
        CanonicalizationProfileId, NamespaceSemanticsId, NegativeProofDigestAlgorithm,
        NegativeProofResourceCaps, NegativeProofSchemeId, NegativeProofSchemeSemanticSubject,
        NegativeProofSubjectKind, NegativeProofValidityRuleId, NormalizedReference,
        ReferenceNormalizerContractId, ReferenceNormalizerSemanticSubject,
        RegisteredAssetResolution, RegisteredNegativeProofError, RegisteredNegativeProofPlan,
        RegisteredNegativeProofScheme, RegisteredReferenceNormalizer, RegisteredResolutionError,
        RegisteredResolverProfile, RegisteredUnavailableCause, ResolverContractId,
        ResolverSemanticSubject, SemanticParameterSchema, StableResourceIdentity,
        StableResourceIdentitySchema, VerifiedNegativeInputProof,
        VerifiedNegativeInputProofBinding, immutable_memory_namespace_absence_v1,
        portable_asset_path_normalizer_v1, same_authority_resolver_v1,
    };
}

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
