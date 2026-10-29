use serde_json::Value;
use std::collections::BTreeMap;

pub(super) const BASELINE_PATH: &str = "docs/research/native-filesystem-resolved-closure.json";
pub(super) const REPOSITORY_BASELINE_PATH: &str =
    "docs/research/native-filesystem-repository-resolved-closure.json";
pub(super) const FACADE_MANIFEST_PATH: &str = "crates/baozi/Cargo.toml";
pub(super) const FACADE_PACKAGE: &str = "baozi";
pub(super) const XTASK_PACKAGE: &str = "baozi-xtask";
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
