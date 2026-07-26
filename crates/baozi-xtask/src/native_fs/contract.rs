use serde_json::Value;
use std::collections::BTreeMap;

pub(super) const BASELINE_PATH: &str = "docs/research/native-filesystem-resolved-closure.json";
pub(super) const REPOSITORY_BASELINE_PATH: &str =
    "docs/research/native-filesystem-repository-resolved-closure.json";
pub(super) const CANDIDATE_TEST_SOURCE_PATH: &str = "crates/baozi-io/src/fs.rs";
pub(super) const REPOSITORY_CANDIDATE_TEST_SOURCE_PATH: &str = "crates/baozi-io/src/repository.rs";
pub(super) const REPOSITORY_CANDIDATE_SURFACE_PATH: &str = "crates/baozi-io/src/lib.rs";
pub(super) const RUNTIME_CANDIDATE_SURFACE_PATH: &str = "crates/baozi-io/src/lib.rs";
pub(super) const FACADE_MANIFEST_PATH: &str = "crates/baozi/Cargo.toml";
pub(super) const FACADE_PACKAGE: &str = "baozi";
pub(super) const FACADE_FORMAT_FEATURE: &str = "all-formats";
pub(super) const XTASK_PACKAGE: &str = "baozi-xtask";
pub(super) const IMPORT_PACKAGE: &str = "baozi-import";
pub(super) const FORMAT_PACKAGE_PREFIX: &str = "baozi-format-";
pub(super) const CRATES_IO_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";
pub(super) const CAP_STD_TARGET: &str = concat!(
    "cfg(any(",
    "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"linux\", target_env = \"gnu\", target_abi = \"\"), ",
    "all(target_arch = \"x86_64\", target_vendor = \"pc\", target_os = \"windows\", target_env = \"msvc\", target_abi = \"\"), ",
    "all(target_arch = \"aarch64\", target_vendor = \"apple\", target_os = \"macos\", target_env = \"\", target_abi = \"\"), ",
    "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"freebsd\", target_env = \"\", target_abi = \"\")))"
);
pub(super) const XTASK_REPOSITORY_TARGET: &str = concat!(
    "cfg(any(",
    "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"linux\", target_env = \"gnu\", target_abi = \"\"), ",
    "all(target_arch = \"x86_64\", target_vendor = \"pc\", target_os = \"windows\", target_env = \"msvc\", target_abi = \"\"), ",
    "all(target_arch = \"aarch64\", target_vendor = \"apple\", target_os = \"macos\", target_env = \"\", target_abi = \"\")))"
);
pub(super) const RUSTIX_TARGET: &str = concat!(
    "cfg(any(",
    "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"linux\", target_env = \"gnu\", target_abi = \"\"), ",
    "all(target_arch = \"aarch64\", target_vendor = \"apple\", target_os = \"macos\", target_env = \"\", target_abi = \"\"), ",
    "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"freebsd\", target_env = \"\", target_abi = \"\")))"
);
pub(super) const NATIVE_FILESYSTEM_PACKAGES: &[&str] =
    &["cap-std", "cap-primitives", "rustix", "cap-fs-ext"];
pub(super) const MAX_BASELINE_DIFFERENCES: usize = 64;
pub(super) const RUNTIME_CANDIDATE_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
];
pub(super) const REPOSITORY_CANDIDATE_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
];
pub(super) const REPOSITORY_OPERATIONAL_CANDIDATE_TARGETS: &[&str] =
    &["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"];
pub(super) const REPOSITORY_FAIL_CLOSED_TARGETS: &[&str] = &["x86_64-pc-windows-msvc"];
pub(super) const XTASK_REPOSITORY_RESOLUTION_TARGET: &str = "x86_64-unknown-linux-gnu";
pub(super) const RELEASE_REGISTERED_TARGETS: &[&str] = &[];
pub(super) const REPOSITORY_PUBLIC_EXPORTS: &[&str] = &[
    "RepositoryCapture",
    "RepositoryCaptureError",
    "RepositoryCaptureErrorKind",
    "RepositoryCaptureStage",
    "RepositoryFile",
    "RepositoryLimit",
    "RepositoryLimitSpec",
    "RepositoryLimits",
    "RepositoryLimitsError",
    "RepositoryObjectKind",
    "RepositoryPath",
    "RepositoryPathError",
    "RepositoryPathErrorCode",
    "RepositoryRoot",
    "RepositoryRootSpec",
    "RepositoryUsage",
    "RepositoryVerifyError",
];
pub(super) const RUNTIME_PUBLIC_EXPORTS: &[&str] =
    &["FileSystemAssetIo", "NativeDirectoryCapability"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CandidateTestPlatform {
    Windows,
    Unix,
}

pub(super) struct RequiredCandidateTest {
    pub(super) name: &'static str,
    pub(super) platform: Option<CandidateTestPlatform>,
}

pub(super) const REQUIRED_CANDIDATE_TESTS: &[RequiredCandidateTest] = &[
    RequiredCandidateTest {
        name: "opens_and_inspects_the_same_regular_file_handle",
        platform: None,
    },
    RequiredCandidateTest {
        name: "rejects_non_directory_roots_and_non_regular_assets",
        platform: None,
    },
    RequiredCandidateTest {
        name: "rejects_symlink_escape",
        platform: None,
    },
    RequiredCandidateTest {
        name: "follows_in_root_symlink_without_reopening_by_host_path",
        platform: None,
    },
    RequiredCandidateTest {
        name: "retained_root_capability_cannot_be_redirected_by_textual_replacement",
        platform: None,
    },
    RequiredCandidateTest {
        name: "opened_file_remains_bound_after_final_path_replacement",
        platform: None,
    },
    RequiredCandidateTest {
        name: "windows_firewall_rejects_ads_device_and_namespace_forms_before_open",
        platform: Some(CandidateTestPlatform::Windows),
    },
    RequiredCandidateTest {
        name: "windows_junction_reparse_escape_is_rejected",
        platform: Some(CandidateTestPlatform::Windows),
    },
    RequiredCandidateTest {
        name: "unix_fifo_is_rejected_without_waiting_for_a_writer",
        platform: Some(CandidateTestPlatform::Unix),
    },
    RequiredCandidateTest {
        name: "unix_socket_and_device_are_rejected",
        platform: Some(CandidateTestPlatform::Unix),
    },
];
pub(super) const CANDIDATE_TEST_MODULE_DIGEST: &str =
    "5cb630243231991cac647453a2227d82a24b5e46d6eabc3892466585a02e4cf7";

pub(super) const REQUIRED_REPOSITORY_CANDIDATE_TESTS: &[RequiredCandidateTest] = &[
    RequiredCandidateTest {
        name: "repository_path_preserves_spelling_and_rejects_non_portable_forms",
        platform: None,
    },
    RequiredCandidateTest {
        name: "repository_limits_accept_exact_boundaries_and_reject_invalid_specs",
        platform: None,
    },
    RequiredCandidateTest {
        name: "reenumeration_scratch_is_bounded_by_original_entry_count",
        platform: None,
    },
    RequiredCandidateTest {
        name: "repository_resource_limits_accept_exact_and_reject_one_over",
        platform: Some(CandidateTestPlatform::Unix),
    },
    RequiredCandidateTest {
        name: "deterministic_capture_records_files_and_empty_directories",
        platform: Some(CandidateTestPlatform::Unix),
    },
    RequiredCandidateTest {
        name: "rejects_duplicate_overlapping_and_case_alias_roots",
        platform: None,
    },
    RequiredCandidateTest {
        name: "rejects_hard_link_aliases",
        platform: Some(CandidateTestPlatform::Unix),
    },
    RequiredCandidateTest {
        name: "verify_detects_added_and_removed_paths",
        platform: Some(CandidateTestPlatform::Unix),
    },
    RequiredCandidateTest {
        name: "verify_detects_content_and_same_content_replacement",
        platform: Some(CandidateTestPlatform::Unix),
    },
    RequiredCandidateTest {
        name: "verify_detects_kind_changes",
        platform: Some(CandidateTestPlatform::Unix),
    },
    RequiredCandidateTest {
        name: "retained_repository_capability_ignores_textual_root_replacement",
        platform: Some(CandidateTestPlatform::Unix),
    },
    RequiredCandidateTest {
        name: "rejects_symlinks_in_roots_and_descendants",
        platform: Some(CandidateTestPlatform::Unix),
    },
    RequiredCandidateTest {
        name: "unix_special_files_fail_closed_without_blocking",
        platform: Some(CandidateTestPlatform::Unix),
    },
    RequiredCandidateTest {
        name: "windows_rejects_case_alias_root_spelling",
        platform: Some(CandidateTestPlatform::Windows),
    },
    RequiredCandidateTest {
        name: "windows_repository_capture_fails_closed_before_traversal",
        platform: Some(CandidateTestPlatform::Windows),
    },
];
pub(super) const REPOSITORY_CANDIDATE_TEST_MODULE_DIGEST: &str =
    "09c89f3d377bba0012ebe727e4b145e7702cb5cc83373de4fff36f2a48310da0";
pub(super) const CANDIDATE_TEST_ALLOWED_MACROS: &[&str] = &[
    "assert",
    "assert_eq",
    "assert_ne",
    "eprintln",
    "format",
    "matches",
    "panic",
    "vec",
];

#[derive(Clone, Copy)]
pub(super) struct DependencyAuditProfile {
    pub(super) name: &'static str,
    pub(super) target: &'static str,
    pub(super) direct_rustix: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DependencyAuditKind {
    Runtime,
    Repository,
}

pub(super) const DEPENDENCY_AUDIT_PROFILES: &[DependencyAuditProfile] = &[
    DependencyAuditProfile {
        name: "linux",
        target: "x86_64-unknown-linux-gnu",
        direct_rustix: true,
    },
    DependencyAuditProfile {
        name: "windows-msvc",
        target: "x86_64-pc-windows-msvc",
        direct_rustix: false,
    },
    DependencyAuditProfile {
        name: "macos",
        target: "aarch64-apple-darwin",
        direct_rustix: true,
    },
    DependencyAuditProfile {
        name: "freebsd",
        target: "x86_64-unknown-freebsd",
        direct_rustix: true,
    },
];

pub(super) const PORTABLE_TARGETS: &[&str] = &["wasm32-unknown-unknown", "wasm32-wasip1"];

pub(super) struct CriticalPackage {
    pub(super) name: &'static str,
    pub(super) version: &'static str,
    pub(super) checksum: &'static str,
}

pub(super) const CRITICAL_PACKAGES: &[CriticalPackage] = &[
    CriticalPackage {
        name: "cap-fs-ext",
        version: "4.0.2",
        checksum: "d78e5a3368ae89b7cb68186411452b4b9fac8b41be9c19bf3f47c2d2c8e36e6b",
    },
    CriticalPackage {
        name: "cap-std",
        version: "4.0.2",
        checksum: "7281235d6e96d3544ca18bba9049be92f4190f8d923e3caef1b5f66cfa752608",
    },
    CriticalPackage {
        name: "cap-primitives",
        version: "4.0.2",
        checksum: "cdadbd7c002d3a484b35243669abdae85a0ebaded5a61117169dc3400f9a7ff0",
    },
    CriticalPackage {
        name: "rustix",
        version: "1.1.4",
        checksum: "b6fe4565b9518b83ef4f91bb47ce29620ca828bd32cb7e408f0062e9930ba190",
    },
];

/// The result of checking the native-filesystem dependency admission contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeFsReport {
    /// Number of target-filtered Cargo resolve graphs checked.
    pub profiles_checked: usize,
    /// Stable, human-readable contract violations.
    pub violations: Vec<String>,
}

/// Resolved metadata profiles required by the repository capability admission fixture.
#[doc(hidden)]
pub struct RepositoryMetadataProfiles<'a> {
    /// Complete all-feature metadata used to inspect every local/path declaration.
    pub local_declarations: &'a Value,
    /// Repository-feature profiles for every dependency-audit target.
    pub repository: &'a BTreeMap<String, Value>,
    /// Facade runtime-candidate profiles for every exact candidate target.
    pub facade: &'a BTreeMap<String, Value>,
    /// The xtask empty-feature profile, which must not reach repository IO.
    pub xtask_empty: &'a Value,
    /// The xtask all-feature profile used to bind the sole workspace dependency edge.
    pub xtask_all: &'a Value,
}
