use serde_json::Value;
use std::collections::BTreeMap;

pub(super) const BASELINE_PATH: &str = "docs/research/native-filesystem-resolved-closure.json";
pub(super) const REPOSITORY_BASELINE_PATH: &str =
    "docs/research/native-filesystem-repository-resolved-closure.json";
pub(super) const CANDIDATE_MANIFEST_PATH: &str = "candidates/baozi-native-io/Cargo.toml";
pub(super) const CANDIDATE_LOCK_PATH: &str = "candidates/baozi-native-io/Cargo.lock";
pub(super) const CANDIDATE_PACKAGE: &str = "baozi-native-io-candidate";
pub(super) const XTASK_PACKAGE: &str = "baozi-xtask";
pub(super) const CRATES_IO_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";
pub(super) const CAP_STD_TARGET: &str = concat!(
    "cfg(any(",
    "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"linux\", target_env = \"gnu\", target_abi = \"\"), ",
    "all(target_arch = \"x86_64\", target_vendor = \"pc\", target_os = \"windows\", target_env = \"msvc\", target_abi = \"\"), ",
    "all(target_arch = \"aarch64\", target_vendor = \"apple\", target_os = \"macos\", target_env = \"\", target_abi = \"\"), ",
    "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"freebsd\", target_env = \"\", target_abi = \"\")))"
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
pub(super) const XTASK_RESOLUTION_TARGET: &str = "x86_64-unknown-linux-gnu";
pub(super) const RELEASE_REGISTERED_TARGETS: &[&str] = &[];

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
        version: "4.0.3",
        checksum: "56ff379b70af8e08307a8f65e7040c7301cb4a572538ade16b4984f0da77847f",
    },
    CriticalPackage {
        name: "cap-std",
        version: "4.0.3",
        checksum: "c1ec78e242cfa2cfe276807ac2ecc00315a6c97786977414bcd1c3963b6c91b8",
    },
    CriticalPackage {
        name: "cap-primitives",
        version: "4.0.3",
        checksum: "8b5f74729fd2f44701d1a8eb47e906cdb3ccd9ec0f02baad85a744b791940b18",
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

/// Independent root-workspace and candidate-workspace metadata, loaded once per profile.
#[doc(hidden)]
pub struct NativeFsMetadataProfiles<'a> {
    /// Root workspace, all features, Linux: local declarations and xtask reachability.
    pub root: &'a Value,
    /// Candidate workspace, no defaults, for every dependency-audit target.
    pub runtime: &'a BTreeMap<String, Value>,
    /// Candidate workspace with repository-capture, for every dependency-audit target.
    pub repository: &'a BTreeMap<String, Value>,
    /// Root workspace, all features, for each portable byte target.
    pub portable: &'a BTreeMap<String, Value>,
}
