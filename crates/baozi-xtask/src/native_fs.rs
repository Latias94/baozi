//! Target-filtered dependency admission for the native filesystem adapter.

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use syn::ext::IdentExt;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{Attribute, Item, ItemMod, Meta, Token, UseTree, Visibility};

const BASELINE_PATH: &str = "docs/research/native-filesystem-resolved-closure.json";
const REPOSITORY_BASELINE_PATH: &str =
    "docs/research/native-filesystem-repository-resolved-closure.json";
const CANDIDATE_TEST_SOURCE_PATH: &str = "crates/baozi-io/src/fs.rs";
const REPOSITORY_CANDIDATE_TEST_SOURCE_PATH: &str = "crates/baozi-io/src/repository.rs";
const REPOSITORY_CANDIDATE_SURFACE_PATH: &str = "crates/baozi-io/src/lib.rs";
const RUNTIME_CANDIDATE_SURFACE_PATH: &str = "crates/baozi-io/src/lib.rs";
const FACADE_MANIFEST_PATH: &str = "crates/baozi/Cargo.toml";
const FACADE_PACKAGE: &str = "baozi";
const FACADE_FORMAT_FEATURE: &str = "all-formats";
const XTASK_PACKAGE: &str = "baozi-xtask";
const IMPORT_PACKAGE: &str = "baozi-import";
const FORMAT_PACKAGE_PREFIX: &str = "baozi-format-";
const CRATES_IO_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";
const CAP_STD_TARGET: &str = concat!(
    "cfg(any(",
    "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"linux\", target_env = \"gnu\", target_abi = \"\"), ",
    "all(target_arch = \"x86_64\", target_vendor = \"pc\", target_os = \"windows\", target_env = \"msvc\", target_abi = \"\"), ",
    "all(target_arch = \"aarch64\", target_vendor = \"apple\", target_os = \"macos\", target_env = \"\", target_abi = \"\"), ",
    "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"freebsd\", target_env = \"\", target_abi = \"\")))"
);
const XTASK_REPOSITORY_TARGET: &str = concat!(
    "cfg(any(",
    "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"linux\", target_env = \"gnu\", target_abi = \"\"), ",
    "all(target_arch = \"x86_64\", target_vendor = \"pc\", target_os = \"windows\", target_env = \"msvc\", target_abi = \"\"), ",
    "all(target_arch = \"aarch64\", target_vendor = \"apple\", target_os = \"macos\", target_env = \"\", target_abi = \"\")))"
);
const RUSTIX_TARGET: &str = concat!(
    "cfg(any(",
    "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"linux\", target_env = \"gnu\", target_abi = \"\"), ",
    "all(target_arch = \"aarch64\", target_vendor = \"apple\", target_os = \"macos\", target_env = \"\", target_abi = \"\"), ",
    "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"freebsd\", target_env = \"\", target_abi = \"\")))"
);
const NATIVE_FILESYSTEM_PACKAGES: &[&str] = &["cap-std", "cap-primitives", "rustix", "cap-fs-ext"];
const MAX_BASELINE_DIFFERENCES: usize = 64;
const RUNTIME_CANDIDATE_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
];
const REPOSITORY_CANDIDATE_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
];
const REPOSITORY_OPERATIONAL_CANDIDATE_TARGETS: &[&str] =
    &["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"];
const REPOSITORY_FAIL_CLOSED_TARGETS: &[&str] = &["x86_64-pc-windows-msvc"];
const XTASK_REPOSITORY_RESOLUTION_TARGET: &str = "x86_64-unknown-linux-gnu";
const RELEASE_REGISTERED_TARGETS: &[&str] = &[];
const REPOSITORY_PUBLIC_EXPORTS: &[&str] = &[
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
const RUNTIME_PUBLIC_EXPORTS: &[&str] = &["FileSystemAssetIo", "NativeDirectoryCapability"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateTestPlatform {
    Windows,
    Unix,
}

struct RequiredCandidateTest {
    name: &'static str,
    platform: Option<CandidateTestPlatform>,
}

const REQUIRED_CANDIDATE_TESTS: &[RequiredCandidateTest] = &[
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
const CANDIDATE_TEST_MODULE_DIGEST: &str =
    "5cb630243231991cac647453a2227d82a24b5e46d6eabc3892466585a02e4cf7";

const REQUIRED_REPOSITORY_CANDIDATE_TESTS: &[RequiredCandidateTest] = &[
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
const REPOSITORY_CANDIDATE_TEST_MODULE_DIGEST: &str =
    "09c89f3d377bba0012ebe727e4b145e7702cb5cc83373de4fff36f2a48310da0";
const CANDIDATE_TEST_ALLOWED_MACROS: &[&str] = &[
    "assert",
    "assert_eq",
    "assert_ne",
    "eprintln",
    "format",
    "matches",
    "panic",
    "vec",
];

fn identifier_name(identifier: &syn::Ident) -> String {
    identifier.unraw().to_string()
}

fn identifier_is(identifier: &syn::Ident, expected: &str) -> bool {
    identifier.unraw() == expected
}

fn path_is_identifier(path: &syn::Path, expected: &str) -> bool {
    path.segments.len() == 1
        && path
            .segments
            .first()
            .is_some_and(|segment| identifier_is(&segment.ident, expected))
}

fn path_ends_in_identifier(path: &syn::Path, expected: &str) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| identifier_is(&segment.ident, expected))
}

fn is_admitted_candidate_implementation_macro(path: &syn::Path) -> bool {
    path.leading_colon.is_some()
        && path.segments.len() == 2
        && path
            .segments
            .first()
            .is_some_and(|segment| identifier_is(&segment.ident, "core"))
        && path.segments.last().is_some_and(|segment| {
            matches!(
                identifier_name(&segment.ident).as_str(),
                "debug_assert" | "matches" | "write"
            )
        })
}

fn token_identifier(token: &str) -> &str {
    token.strip_prefix("r#").unwrap_or(token)
}

#[derive(Clone, Copy)]
struct DependencyAuditProfile {
    name: &'static str,
    target: &'static str,
    direct_rustix: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DependencyAuditKind {
    Runtime,
    Repository,
}

const DEPENDENCY_AUDIT_PROFILES: &[DependencyAuditProfile] = &[
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

const PORTABLE_TARGETS: &[&str] = &["wasm32-unknown-unknown", "wasm32-wasip1"];

struct CriticalPackage {
    name: &'static str,
    version: &'static str,
    checksum: &'static str,
}

const CRITICAL_PACKAGES: &[CriticalPackage] = &[
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

/// Checks the checked-in manifest and target-filtered resolved closure.
pub fn check_repository(repository_root: &Path) -> NativeFsReport {
    let baseline = match read_json(&repository_root.join(BASELINE_PATH), "native-fs baseline") {
        Ok(value) => value,
        Err(violation) => {
            return NativeFsReport {
                profiles_checked: 0,
                violations: vec![violation],
            };
        }
    };
    let repository_baseline = match read_json(
        &repository_root.join(REPOSITORY_BASELINE_PATH),
        "native-fs repository baseline",
    ) {
        Ok(value) => value,
        Err(violation) => {
            return NativeFsReport {
                profiles_checked: 0,
                violations: vec![violation],
            };
        }
    };
    let manifest_metadata = match load_metadata(
        repository_root,
        "crates/baozi-io/Cargo.toml",
        None,
        None,
        true,
    ) {
        Ok(value) => value,
        Err(violation) => {
            return NativeFsReport {
                profiles_checked: 0,
                violations: vec![violation],
            };
        }
    };
    let local_declaration_metadata =
        match load_metadata_all_features(repository_root, "Cargo.toml", None, false) {
            Ok(value) => value,
            Err(violation) => {
                return NativeFsReport {
                    profiles_checked: 0,
                    violations: vec![violation],
                };
            }
        };
    let source_metadata = match load_metadata(
        repository_root,
        FACADE_MANIFEST_PATH,
        None,
        Some(FACADE_FORMAT_FEATURE),
        false,
    ) {
        Ok(value) => value,
        Err(violation) => {
            return NativeFsReport {
                profiles_checked: 0,
                violations: vec![violation],
            };
        }
    };
    let lockfile = match read_lockfile(repository_root) {
        Ok(value) => value,
        Err(violation) => {
            return NativeFsReport {
                profiles_checked: 0,
                violations: vec![violation],
            };
        }
    };

    let mut target_metadata = BTreeMap::new();
    let mut repository_target_metadata = BTreeMap::new();
    let mut facade_candidate_target_metadata = BTreeMap::new();
    let mut xtask_empty_metadata = None;
    let mut xtask_all_metadata = None;
    let mut violations = Vec::new();
    check_format_import_production_sources(repository_root, &source_metadata, &mut violations);
    for profile in DEPENDENCY_AUDIT_PROFILES {
        match load_metadata(
            repository_root,
            "crates/baozi-io/Cargo.toml",
            Some(profile.target),
            Some("fs-candidate"),
            false,
        ) {
            Ok(value) => {
                target_metadata.insert(profile.target.to_owned(), value);
            }
            Err(violation) => violations.push(violation),
        }
    }
    for target in REPOSITORY_CANDIDATE_TARGETS {
        match load_metadata(
            repository_root,
            FACADE_MANIFEST_PATH,
            Some(target),
            Some("native-fs-candidate"),
            false,
        ) {
            Ok(value) => {
                facade_candidate_target_metadata.insert((*target).to_owned(), value);
            }
            Err(violation) => violations.push(violation),
        }
    }
    match load_metadata(
        repository_root,
        "crates/baozi-xtask/Cargo.toml",
        Some(XTASK_REPOSITORY_RESOLUTION_TARGET),
        None,
        false,
    ) {
        Ok(value) => xtask_empty_metadata = Some(value),
        Err(violation) => violations.push(violation),
    }
    match load_metadata_all_features(
        repository_root,
        "crates/baozi-xtask/Cargo.toml",
        Some(XTASK_REPOSITORY_RESOLUTION_TARGET),
        false,
    ) {
        Ok(value) => xtask_all_metadata = Some(value),
        Err(violation) => violations.push(violation),
    }
    for profile in DEPENDENCY_AUDIT_PROFILES {
        match load_metadata(
            repository_root,
            "crates/baozi-io/Cargo.toml",
            Some(profile.target),
            Some("repository-fs-candidate"),
            false,
        ) {
            Ok(value) => {
                repository_target_metadata.insert(profile.target.to_owned(), value);
            }
            Err(violation) => violations.push(violation),
        }
    }
    for target in PORTABLE_TARGETS {
        match load_metadata(
            repository_root,
            "crates/baozi/Cargo.toml",
            Some(target),
            Some("all-formats"),
            false,
        ) {
            Ok(value) => {
                target_metadata.insert((*target).to_owned(), value);
            }
            Err(violation) => violations.push(violation),
        }
    }

    if !violations.is_empty() {
        violations.sort_unstable();
        return NativeFsReport {
            profiles_checked: target_metadata.len()
                + repository_target_metadata.len()
                + facade_candidate_target_metadata.len()
                + usize::from(xtask_empty_metadata.is_some())
                + usize::from(xtask_all_metadata.is_some()),
            violations,
        };
    }

    let xtask_empty_metadata =
        xtask_empty_metadata.expect("successful metadata load must populate the empty profile");
    let xtask_all_metadata =
        xtask_all_metadata.expect("successful metadata load must populate the all-feature profile");
    let mut report = check_metadata_fixture_with_repository(
        &manifest_metadata,
        &lockfile,
        &baseline,
        &repository_baseline,
        &target_metadata,
        RepositoryMetadataProfiles {
            local_declarations: &local_declaration_metadata,
            repository: &repository_target_metadata,
            facade: &facade_candidate_target_metadata,
            xtask_empty: &xtask_empty_metadata,
            xtask_all: &xtask_all_metadata,
        },
    );
    match fs::read_to_string(repository_root.join(CANDIDATE_TEST_SOURCE_PATH)) {
        Ok(source) => report
            .violations
            .extend(candidate_test_source_violations(&source)),
        Err(error) => report.violations.push(format!(
            "native-fs candidate tests: failed to read {CANDIDATE_TEST_SOURCE_PATH}: {error}"
        )),
    }
    report
        .violations
        .extend(candidate_test_crate_ancestor_violations(
            repository_root,
            "native-fs candidate tests",
        ));
    check_candidate_implementation_cfg_sources(repository_root, &mut report.violations);
    report
        .violations
        .extend(repository_candidate_test_tree_violations(repository_root));
    match fs::read_to_string(repository_root.join(RUNTIME_CANDIDATE_SURFACE_PATH)) {
        Ok(source) => report
            .violations
            .extend(runtime_candidate_surface_violations(&source)),
        Err(error) => report.violations.push(format!(
            "native-fs runtime candidate surface: failed to read {RUNTIME_CANDIDATE_SURFACE_PATH}: {error}"
        )),
    }
    match fs::read_to_string(repository_root.join(REPOSITORY_CANDIDATE_SURFACE_PATH)) {
        Ok(source) => report
            .violations
            .extend(repository_candidate_surface_violations(&source)),
        Err(error) => report.violations.push(format!(
            "native-fs repository candidate surface: failed to read {REPOSITORY_CANDIDATE_SURFACE_PATH}: {error}"
        )),
    }
    report.violations.sort_unstable();
    report.violations.dedup();
    report
}

/// Rejects source patterns that can turn unavailable fixtures into passing candidate tests.
#[doc(hidden)]
pub fn candidate_test_source_violations(source: &str) -> Vec<String> {
    candidate_test_source_violations_for_contract(
        source,
        REQUIRED_CANDIDATE_TESTS,
        CANDIDATE_TEST_MODULE_DIGEST,
        "native-fs candidate tests",
    )
}

/// Rejects no-skip and early-success patterns in repository candidate tests.
#[doc(hidden)]
pub fn repository_candidate_test_source_violations(source: &str) -> Vec<String> {
    candidate_test_source_violations_for_contract(
        source,
        REQUIRED_REPOSITORY_CANDIDATE_TESTS,
        REPOSITORY_CANDIDATE_TEST_MODULE_DIGEST,
        "native-fs repository candidate tests",
    )
}

/// Loads the repository candidate's out-of-line test module and checks its contract.
#[doc(hidden)]
pub fn repository_candidate_test_tree_violations(repository_root: &Path) -> Vec<String> {
    const LABEL: &str = "native-fs repository candidate tests";
    let entry_path = repository_root.join(REPOSITORY_CANDIDATE_TEST_SOURCE_PATH);
    let entry_source = match fs::read_to_string(&entry_path) {
        Ok(source) => source,
        Err(error) => {
            return vec![format!(
                "{LABEL}: failed to read {REPOSITORY_CANDIDATE_TEST_SOURCE_PATH}: {error}"
            )];
        }
    };
    let entry = match syn::parse_file(&entry_source) {
        Ok(file) => file,
        Err(error) => {
            return vec![format!(
                "{LABEL}: failed to parse {REPOSITORY_CANDIDATE_TEST_SOURCE_PATH}: {error}"
            )];
        }
    };
    let mut ancestor_violations = Vec::new();
    check_candidate_test_ancestor_macro_bindings(&entry, LABEL, &mut ancestor_violations);
    ancestor_violations.extend(candidate_test_crate_ancestor_violations(
        repository_root,
        LABEL,
    ));
    let matching: Vec<_> = entry
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Mod(module) if identifier_is(&module.ident, "tests") => Some(module),
            _ => None,
        })
        .collect();
    let test_module = match matching.as_slice() {
        [module] => *module,
        _ => {
            return vec![format!(
                "{LABEL}: {REPOSITORY_CANDIDATE_TEST_SOURCE_PATH} must declare exactly one tests module, found {}",
                matching.len()
            )];
        }
    };
    if test_module.content.is_some()
        || !matches!(test_module.vis, Visibility::Inherited)
        || test_module.attrs.len() != 1
        || !exact_cfg_test_attribute(&test_module.attrs[0])
        || module_source_override(&test_module.attrs).is_some()
    {
        return vec![format!(
            "{LABEL}: {REPOSITORY_CANDIDATE_TEST_SOURCE_PATH} tests module must be exactly #[cfg(test)] mod tests;"
        )];
    }
    let module_directory = module_directory_for_file(&entry_path);
    let Some(test_path) = resolve_module_file(&module_directory, test_module) else {
        return vec![format!(
            "{LABEL}: cannot resolve tests module from {REPOSITORY_CANDIDATE_TEST_SOURCE_PATH}"
        )];
    };
    let test_source = match fs::read_to_string(&test_path) {
        Ok(source) => source,
        Err(error) => {
            return vec![format!(
                "{LABEL}: failed to read {}: {error}",
                display_source_path(repository_root, &test_path)
            )];
        }
    };
    let synthetic = format!("#[cfg(test)]\nmod tests {{\n{test_source}\n}}\n");
    let mut violations = repository_candidate_test_source_violations(&synthetic);
    violations.extend(ancestor_violations);
    violations.sort_unstable();
    violations.dedup();
    violations
}

fn candidate_test_crate_ancestor_violations(repository_root: &Path, label: &str) -> Vec<String> {
    let path = repository_root.join(REPOSITORY_CANDIDATE_SURFACE_PATH);
    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) => {
            return vec![format!(
                "{label}: failed to read {REPOSITORY_CANDIDATE_SURFACE_PATH}: {error}"
            )];
        }
    };
    let file = match syn::parse_file(&source) {
        Ok(file) => file,
        Err(error) => {
            return vec![format!(
                "{label}: failed to parse {REPOSITORY_CANDIDATE_SURFACE_PATH}: {error}"
            )];
        }
    };
    let mut violations = Vec::new();
    check_candidate_test_ancestor_macro_bindings(&file, label, &mut violations);
    let mut visited = BTreeSet::new();
    scan_candidate_macro_hazards(
        repository_root,
        &path,
        &file,
        label,
        &mut visited,
        &mut violations,
    );
    violations.sort_unstable();
    violations.dedup();
    violations
}

fn scan_candidate_macro_hazards(
    repository_root: &Path,
    source_path: &Path,
    file: &syn::File,
    label: &str,
    visited: &mut BTreeSet<PathBuf>,
    violations: &mut Vec<String>,
) {
    let canonical = match fs::canonicalize(source_path) {
        Ok(path) => path,
        Err(error) => {
            violations.push(format!(
                "{label}: cannot inspect {} while checking macro bindings: {error}",
                display_source_path(repository_root, source_path)
            ));
            return;
        }
    };
    if !visited.insert(canonical) {
        return;
    }
    if file.attrs.iter().any(|attribute| {
        path_is_identifier(attribute.path(), "macro_use")
            || path_is_identifier(attribute.path(), "macro_export")
    }) {
        violations.push(format!(
            "{label}: {} has a crate/module macro import or export that can alter candidate-test bindings",
            display_source_path(repository_root, source_path)
        ));
    }
    scan_candidate_macro_items(
        repository_root,
        source_path,
        &file.items,
        label,
        visited,
        violations,
    );
}

fn scan_candidate_macro_items(
    repository_root: &Path,
    source_path: &Path,
    items: &[Item],
    label: &str,
    visited: &mut BTreeSet<PathBuf>,
    violations: &mut Vec<String>,
) {
    let module_directory = module_directory_for_file(source_path);
    for item in items {
        if item_attributes(item).iter().any(|attribute| {
            path_is_identifier(attribute.path(), "macro_use")
                || path_is_identifier(attribute.path(), "macro_export")
        }) {
            violations.push(format!(
                "{label}: {} contains an item macro import/export that can alter candidate-test bindings",
                display_source_path(repository_root, source_path)
            ));
        }
        match item {
            Item::Macro(item_macro) if item_macro.ident.is_none() => {
                let invocation = item_macro
                    .mac
                    .path
                    .segments
                    .last()
                    .map(|segment| identifier_name(&segment.ident))
                    .unwrap_or_else(|| "<empty-path>".to_owned());
                if invocation != "compile_error" {
                    violations.push(format!(
                        "{label}: {} invokes item macro {invocation}! that can introduce candidate-test bindings",
                        display_source_path(repository_root, source_path)
                    ));
                }
            }
            Item::ExternCrate(item_extern_crate)
                if item_extern_crate
                    .attrs
                    .iter()
                    .any(|attribute| path_is_identifier(attribute.path(), "macro_use")) =>
            {
                violations.push(format!(
                    "{label}: {} imports macros with extern crate",
                    display_source_path(repository_root, source_path)
                ));
            }
            Item::Mod(module) => {
                if let Some(attribute) = module_source_override(&module.attrs) {
                    violations.push(format!(
                        "{label}: module {} in {} uses forbidden {attribute} source redirection while checking macro bindings",
                        module.ident,
                        display_source_path(repository_root, source_path)
                    ));
                    continue;
                }
                if let Some((_, nested_items)) = &module.content {
                    scan_candidate_macro_items(
                        repository_root,
                        source_path,
                        nested_items,
                        label,
                        visited,
                        violations,
                    );
                    continue;
                }
                let Some(module_file) = resolve_module_file(&module_directory, module) else {
                    violations.push(format!(
                        "{label}: cannot resolve module {} from {} while checking macro bindings",
                        module.ident,
                        display_source_path(repository_root, source_path)
                    ));
                    continue;
                };
                let module_source = match fs::read_to_string(&module_file) {
                    Ok(source) => source,
                    Err(error) => {
                        violations.push(format!(
                            "{label}: failed to read {} while checking macro bindings: {error}",
                            display_source_path(repository_root, &module_file)
                        ));
                        continue;
                    }
                };
                let module_file_ast = match syn::parse_file(&module_source) {
                    Ok(file) => file,
                    Err(error) => {
                        violations.push(format!(
                            "{label}: failed to parse {} while checking macro bindings: {error}",
                            display_source_path(repository_root, &module_file)
                        ));
                        continue;
                    }
                };
                scan_candidate_macro_hazards(
                    repository_root,
                    &module_file,
                    &module_file_ast,
                    label,
                    visited,
                    violations,
                );
            }
            _ => {}
        }
    }
}

/// Verifies that the repository module and its public re-export compile on the exact candidate set.
#[doc(hidden)]
pub fn repository_candidate_surface_violations(source: &str) -> Vec<String> {
    const LABEL: &str = "native-fs repository candidate surface";
    let file = match syn::parse_file(source) {
        Ok(file) => file,
        Err(error) => return vec![format!("{LABEL}: failed to parse source: {error}")],
    };
    let mut violations = Vec::new();
    for attribute in &file.attrs {
        if ["cfg", "cfg_attr", "path"]
            .iter()
            .any(|name| path_is_identifier(attribute.path(), name))
        {
            violations.push(format!(
                "{LABEL}: crate-level {} attribute is forbidden",
                identifier_name(
                    &attribute
                        .path()
                        .segments
                        .last()
                        .expect("an attribute path is non-empty")
                        .ident
                )
            ));
        }
    }
    let modules = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Mod(module) if identifier_is(&module.ident, "repository") => Some(module),
            _ => None,
        })
        .collect::<Vec<_>>();
    let reexports = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Use(item)
                if matches!(item.vis, Visibility::Public(_))
                    && use_tree_root_identifier(&item.tree)
                        .is_some_and(|identifier| identifier_is(identifier, "repository")) =>
            {
                Some(item)
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    let module = match modules.as_slice() {
        [module] => Some(*module),
        _ => {
            violations.push(format!(
                "{LABEL}: expected exactly one repository module, found {}",
                modules.len()
            ));
            None
        }
    };
    if let Some(module) = module {
        if !matches!(module.vis, Visibility::Inherited)
            || module.content.is_some()
            || module.attrs.len() != 1
        {
            violations.push(format!(
                "{LABEL}: repository module must be a private out-of-line module with exactly one cfg attribute and no path override"
            ));
        }
        check_repository_surface_cfg("module", &module.attrs, &mut violations);
    }

    let reexport = match reexports.as_slice() {
        [reexport] => Some(*reexport),
        _ => {
            violations.push(format!(
                "{LABEL}: expected exactly one repository public re-export, found {}",
                reexports.len()
            ));
            None
        }
    };
    if let Some(reexport) = reexport {
        if reexport.attrs.len() != 1 {
            violations.push(format!(
                "{LABEL}: repository public re-export must have exactly one cfg attribute"
            ));
        }
        check_repository_surface_cfg("public re-export", &reexport.attrs, &mut violations);
        match repository_reexport_names(&reexport.tree) {
            Ok(actual) => {
                let expected = string_set(REPOSITORY_PUBLIC_EXPORTS);
                if actual != expected {
                    violations.push(format!(
                        "{LABEL}: repository public re-export set must be {expected:?}, found {actual:?}"
                    ));
                }
            }
            Err(reason) => violations.push(format!(
                "{LABEL}: repository public re-export is invalid: {reason}"
            )),
        }
    }
    check_repository_target_partition(&mut violations);
    violations.sort_unstable();
    violations.dedup();
    violations
}

/// Verifies that the runtime filesystem module and its public re-export compile on the exact candidate set.
#[doc(hidden)]
pub fn runtime_candidate_surface_violations(source: &str) -> Vec<String> {
    const LABEL: &str = "native-fs runtime candidate surface";
    let file = match syn::parse_file(source) {
        Ok(file) => file,
        Err(error) => return vec![format!("{LABEL}: failed to parse source: {error}")],
    };
    let mut violations = Vec::new();
    for attribute in &file.attrs {
        if ["cfg", "cfg_attr", "path"]
            .iter()
            .any(|name| path_is_identifier(attribute.path(), name))
        {
            violations.push(format!(
                "{LABEL}: crate-level {} attribute is forbidden",
                identifier_name(
                    &attribute
                        .path()
                        .segments
                        .last()
                        .expect("an attribute path is non-empty")
                        .ident
                )
            ));
        }
    }
    let modules = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Mod(module) if identifier_is(&module.ident, "fs") => Some(module),
            _ => None,
        })
        .collect::<Vec<_>>();
    let reexports = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Use(item)
                if matches!(item.vis, Visibility::Public(_))
                    && use_tree_root_identifier(&item.tree)
                        .is_some_and(|identifier| identifier_is(identifier, "fs")) =>
            {
                Some(item)
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    match modules.as_slice() {
        [module] => {
            if !matches!(module.vis, Visibility::Inherited)
                || module.content.is_some()
                || module.attrs.len() != 1
            {
                violations.push(format!(
                    "{LABEL}: filesystem module must be a private out-of-line module with exactly one cfg attribute and no path override"
                ));
            }
            match runtime_candidate_cfg_targets(&module.attrs) {
                Ok(targets) => {
                    let expected = string_set(RUNTIME_CANDIDATE_TARGETS);
                    if targets != expected {
                        violations.push(format!(
                            "{LABEL}: filesystem module cfg target set must be {expected:?}, found {targets:?}"
                        ));
                    }
                }
                Err(reason) => violations.push(format!(
                    "{LABEL}: filesystem module cfg is invalid: {reason}"
                )),
            }
        }
        _ => violations.push(format!(
            "{LABEL}: expected exactly one filesystem module, found {}",
            modules.len()
        )),
    }

    match reexports.as_slice() {
        [reexport] => {
            if reexport.attrs.len() != 1 {
                violations.push(format!(
                    "{LABEL}: filesystem public re-export must have exactly one cfg attribute"
                ));
            }
            match runtime_candidate_cfg_targets(&reexport.attrs) {
                Ok(targets) => {
                    let expected = string_set(RUNTIME_CANDIDATE_TARGETS);
                    if targets != expected {
                        violations.push(format!(
                            "{LABEL}: filesystem public re-export cfg target set must be {expected:?}, found {targets:?}"
                        ));
                    }
                }
                Err(reason) => violations.push(format!(
                    "{LABEL}: filesystem public re-export cfg is invalid: {reason}"
                )),
            }
            match runtime_reexport_names(&reexport.tree) {
                Ok(actual) => {
                    let expected = string_set(RUNTIME_PUBLIC_EXPORTS);
                    if actual != expected {
                        violations.push(format!(
                            "{LABEL}: filesystem public re-export set must be {expected:?}, found {actual:?}"
                        ));
                    }
                }
                Err(reason) => violations.push(format!(
                    "{LABEL}: filesystem public re-export is invalid: {reason}"
                )),
            }
        }
        _ => violations.push(format!(
            "{LABEL}: expected exactly one filesystem public re-export, found {}",
            reexports.len()
        )),
    }
    violations.sort_unstable();
    violations.dedup();
    violations
}

fn runtime_reexport_names(tree: &UseTree) -> Result<BTreeSet<String>, String> {
    let UseTree::Path(root) = tree else {
        return Err("root must be fs::{...}".to_owned());
    };
    if !identifier_is(&root.ident, "fs") {
        return Err("root must be fs::{...}".to_owned());
    }
    let UseTree::Group(group) = root.tree.as_ref() else {
        return Err("filesystem re-export must use one explicit group".to_owned());
    };
    let mut names = BTreeSet::new();
    for item in &group.items {
        let UseTree::Name(name) = item else {
            return Err("aliases, globs, and nested paths are forbidden".to_owned());
        };
        let name = identifier_name(&name.ident);
        if !names.insert(name.clone()) {
            return Err(format!("duplicate re-export {name}"));
        }
    }
    Ok(names)
}

fn check_repository_surface_cfg(
    kind: &str,
    attributes: &[Attribute],
    violations: &mut Vec<String>,
) {
    const LABEL: &str = "native-fs repository candidate surface";
    match repository_candidate_cfg_targets(attributes) {
        Ok(targets) => {
            let expected = string_set(REPOSITORY_CANDIDATE_TARGETS);
            if targets != expected {
                violations.push(format!(
                        "{LABEL}: repository {kind} cfg target set must be {expected:?}, found {targets:?}"
                    ));
            }
        }
        Err(reason) => violations.push(format!(
            "{LABEL}: repository {kind} cfg is invalid: {reason}"
        )),
    }
}

fn repository_reexport_names(tree: &UseTree) -> Result<BTreeSet<String>, String> {
    let UseTree::Path(root) = tree else {
        return Err("root must be repository::{...}".to_owned());
    };
    if !identifier_is(&root.ident, "repository") {
        return Err("root must be repository::{...}".to_owned());
    }
    let UseTree::Group(group) = root.tree.as_ref() else {
        return Err("repository re-export must use one explicit group".to_owned());
    };
    let mut names = BTreeSet::new();
    for item in &group.items {
        let UseTree::Name(name) = item else {
            return Err("aliases, globs, and nested paths are forbidden".to_owned());
        };
        let name = identifier_name(&name.ident);
        if !names.insert(name.clone()) {
            return Err(format!("duplicate re-export {name}"));
        }
    }
    Ok(names)
}

fn use_tree_root_identifier(tree: &UseTree) -> Option<&syn::Ident> {
    match tree {
        UseTree::Path(path) => Some(&path.ident),
        UseTree::Name(name) => Some(&name.ident),
        UseTree::Rename(rename) => Some(&rename.ident),
        UseTree::Group(_) | UseTree::Glob(_) => None,
    }
}

fn repository_candidate_cfg_targets(attributes: &[Attribute]) -> Result<BTreeSet<String>, String> {
    candidate_surface_cfg_targets(attributes, "repository-fs-candidate", "repository")
}

fn runtime_candidate_cfg_targets(attributes: &[Attribute]) -> Result<BTreeSet<String>, String> {
    candidate_surface_cfg_targets(attributes, "fs-candidate", "runtime")
}

fn candidate_surface_cfg_targets(
    attributes: &[Attribute],
    expected_feature: &str,
    kind: &str,
) -> Result<BTreeSet<String>, String> {
    if attributes
        .iter()
        .any(|attribute| path_is_identifier(attribute.path(), "cfg_attr"))
    {
        return Err("cfg_attr is forbidden".to_owned());
    }
    let cfg_attributes = attributes
        .iter()
        .filter(|attribute| path_is_identifier(attribute.path(), "cfg"))
        .collect::<Vec<_>>();
    let attribute = match cfg_attributes.as_slice() {
        [attribute] => *attribute,
        _ => {
            return Err(format!(
                "expected exactly one cfg attribute, found {}",
                cfg_attributes.len()
            ));
        }
    };
    let Meta::List(cfg) = &attribute.meta else {
        return Err("cfg attribute must be a list".to_owned());
    };
    let cfg_arguments = parse_meta_arguments(cfg).ok_or_else(|| "cannot parse cfg".to_owned())?;
    if cfg_arguments.len() != 1 {
        return Err("cfg must contain one all(...) expression".to_owned());
    }
    let Some(Meta::List(outer)) = cfg_arguments.first() else {
        return Err("cfg must contain one all(...) expression".to_owned());
    };
    if !path_is_identifier(&outer.path, "all") {
        return Err("cfg must contain one all(...) expression".to_owned());
    }
    let outer_arguments = parse_meta_arguments(outer)
        .ok_or_else(|| "cannot parse outer all(...) expression".to_owned())?;
    if outer_arguments.len() != 2 {
        return Err(
            "outer all(...) must contain exactly feature and target-set clauses".to_owned(),
        );
    }

    let mut feature_seen = false;
    let mut targets = None;
    for argument in &outer_arguments {
        if meta_name_value_string(argument, "feature").as_deref() == Some(expected_feature) {
            if feature_seen {
                return Err(format!("{kind} feature clause is duplicated"));
            }
            feature_seen = true;
            continue;
        }
        let Meta::List(any) = argument else {
            return Err("unexpected repository cfg clause".to_owned());
        };
        if !path_is_identifier(&any.path, "any") || targets.is_some() {
            return Err("target clause must be one any(...) expression".to_owned());
        }
        let target_arguments = parse_meta_arguments(any)
            .ok_or_else(|| "cannot parse target any(...) expression".to_owned())?;
        let mut parsed_targets = BTreeSet::new();
        for target in &target_arguments {
            let triple = repository_cfg_target_triple(target)?;
            if !parsed_targets.insert(triple.clone()) {
                return Err(format!("target clause contains duplicate {triple:?}"));
            }
        }
        targets = Some(parsed_targets);
    }
    if !feature_seen {
        return Err(format!("{kind} feature clause is missing or inexact"));
    }
    targets.ok_or_else(|| "target any(...) clause is missing".to_owned())
}

fn repository_cfg_target_triple(meta: &Meta) -> Result<String, String> {
    let Meta::List(all) = meta else {
        return Err("each target must be an all(...) expression".to_owned());
    };
    if !path_is_identifier(&all.path, "all") {
        return Err("each target must be an all(...) expression".to_owned());
    }
    let arguments =
        parse_meta_arguments(all).ok_or_else(|| "cannot parse target all(...)".to_owned())?;
    let mut fields = BTreeMap::new();
    for field in [
        "target_arch",
        "target_vendor",
        "target_os",
        "target_env",
        "target_abi",
    ] {
        let matching = arguments
            .iter()
            .filter_map(|argument| meta_name_value_string(argument, field))
            .collect::<Vec<_>>();
        match matching.as_slice() {
            [value] => {
                fields.insert(field, value.clone());
            }
            _ => {
                return Err(format!(
                    "target all(...) must contain exactly one {field} clause"
                ));
            }
        }
    }
    if arguments.len() != fields.len() {
        return Err("target all(...) contains an unsupported clause".to_owned());
    }
    let triple = match (
        fields["target_arch"].as_str(),
        fields["target_vendor"].as_str(),
        fields["target_os"].as_str(),
        fields["target_env"].as_str(),
        fields["target_abi"].as_str(),
    ) {
        ("x86_64", "unknown", "linux", "gnu", "") => "x86_64-unknown-linux-gnu",
        ("x86_64", "pc", "windows", "msvc", "") => "x86_64-pc-windows-msvc",
        ("aarch64", "apple", "macos", "", "") => "aarch64-apple-darwin",
        values => return Err(format!("unsupported exact target tuple {values:?}")),
    };
    Ok(triple.to_owned())
}

fn meta_name_value_string(meta: &Meta, expected_path: &str) -> Option<String> {
    let Meta::NameValue(name_value) = meta else {
        return None;
    };
    if !path_is_identifier(&name_value.path, expected_path) {
        return None;
    }
    let syn::Expr::Lit(expression) = &name_value.value else {
        return None;
    };
    let syn::Lit::Str(value) = &expression.lit else {
        return None;
    };
    Some(value.value())
}

fn string_set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn check_repository_target_partition(violations: &mut Vec<String>) {
    let candidates = string_set(REPOSITORY_CANDIDATE_TARGETS);
    let operational = string_set(REPOSITORY_OPERATIONAL_CANDIDATE_TARGETS);
    let fail_closed = string_set(REPOSITORY_FAIL_CLOSED_TARGETS);
    let overlap = operational
        .intersection(&fail_closed)
        .cloned()
        .collect::<Vec<_>>();
    if !overlap.is_empty() {
        violations.push(format!(
            "native-fs repository targets: operational and fail-closed sets overlap at {overlap:?}"
        ));
    }
    let qualified = operational.union(&fail_closed).cloned().collect();
    if candidates != qualified {
        violations.push(format!(
            "native-fs repository targets: operational and fail-closed union must be {candidates:?}, found {qualified:?}"
        ));
    }
}

fn candidate_test_source_violations_for_contract(
    source: &str,
    required_tests: &[RequiredCandidateTest],
    expected_module_digest: &str,
    label: &str,
) -> Vec<String> {
    let mut violations = Vec::new();
    for (marker, description) in [
        ("SKIP", "a textual skip marker"),
        ("BAOZI_TEST_", "an environment-dependent external fixture"),
    ] {
        if source.contains(marker) {
            violations.push(format!("{label}: {description} is forbidden ({marker:?})"));
        }
    }
    for (line_index, line) in source.lines().enumerate() {
        if line.contains("fs::rename(") && line.contains(".is_ok()") {
            violations.push(format!(
                "{label}: line {} must not turn an fs::rename error into a passing test with .is_ok()",
                line_index + 1
            ));
        }
    }
    let file = match syn::parse_file(source) {
        Ok(file) => file,
        Err(error) => {
            violations.push(format!(
                "{label}: failed to parse candidate source: {error}"
            ));
            violations.sort_unstable();
            violations.dedup();
            return violations;
        }
    };
    check_candidate_test_ancestor_macro_bindings(&file, label, &mut violations);
    match candidate_test_module_digest(&file) {
        Ok(actual) if actual == expected_module_digest => {}
        Ok(actual) => violations.push(format!(
            "{label}: normalized test-module digest must be {expected_module_digest}, found {actual}"
        )),
        Err(reason) => violations.push(format!("{label}: {reason}")),
    }
    let mut collector = CandidateTestCollector::default();
    collector.visit_file(&file);
    for test in &collector.tests {
        check_candidate_test(test, label, &mut violations);
    }
    for required in required_tests {
        let matches = collector
            .tests
            .iter()
            .filter(|test| identifier_is(&test.item.sig.ident, required.name))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => violations.push(format!("{label}: missing required test {}", required.name)),
            [test] => {
                match candidate_test_platform(&test.item.attrs) {
                    Ok(actual) if actual == required.platform => {}
                    Ok(actual) => violations.push(format!(
                        "{label}: required test {} has platform {actual:?}, expected {:?}",
                        required.name, required.platform
                    )),
                    Err(()) => violations.push(format!(
                        "{label}: required test {} has an unsupported conditional attribute",
                        required.name
                    )),
                }
                check_required_candidate_test_body(test, label, &mut violations);
            }
            _ => violations.push(format!(
                "{label}: required test {} is defined more than once",
                required.name
            )),
        }
    }
    violations.sort_unstable();
    violations.dedup();
    violations
}

fn candidate_test_module_digest(file: &syn::File) -> Result<String, String> {
    let modules = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Mod(module) if identifier_is(&module.ident, "tests") => Some(module),
            _ => None,
        })
        .collect::<Vec<_>>();
    let module = match modules.as_slice() {
        [module] => *module,
        _ => {
            return Err(format!(
                "expected exactly one top-level tests module for digesting, found {}",
                modules.len()
            ));
        }
    };
    if module.content.is_none()
        || !matches!(module.vis, Visibility::Inherited)
        || module.attrs.len() != 1
        || !exact_cfg_test_attribute(&module.attrs[0])
    {
        return Err(
            "top-level tests module must be exactly an inline private #[cfg(test)] mod tests"
                .to_owned(),
        );
    }
    let mut normalized = TokenStream::new();
    module.to_tokens(&mut normalized);
    let digest = Sha256::digest(normalized.to_string().as_bytes());
    Ok(format!("{digest:x}"))
}

fn check_candidate_test_ancestor_macro_bindings(
    file: &syn::File,
    label: &str,
    violations: &mut Vec<String>,
) {
    for attribute in &file.attrs {
        if path_is_identifier(attribute.path(), "macro_use") {
            violations.push(format!(
                "{label}: ancestor #[macro_use] can shadow candidate-test macros"
            ));
        }
    }

    for item in &file.items {
        if matches!(
            item,
            Item::Mod(module) if identifier_is(&module.ident, "tests")
        ) {
            let Item::Mod(module) = item else {
                unreachable!("the match above requires a module");
            };
            check_candidate_test_module_export_hazards(module, label, violations);
            continue;
        }
        if let Item::Mod(module) = item {
            check_candidate_test_module_export_hazards(module, label, violations);
        }
        if item_attributes(item)
            .iter()
            .any(|attribute| path_is_identifier(attribute.path(), "macro_use"))
        {
            violations.push(format!(
                "{label}: ancestor #[macro_use] can shadow candidate-test macros"
            ));
        }
        match item {
            Item::Macro(item_macro) => {
                if let Some(identifier) = &item_macro.ident {
                    let name = identifier_name(identifier);
                    if is_candidate_test_ancestor_guard_macro(&name) {
                        violations.push(format!(
                            "{label}: ancestor scope shadows candidate-test macro {name}! with a macro declaration"
                        ));
                    }
                } else {
                    let invocation = item_macro
                        .mac
                        .path
                        .segments
                        .last()
                        .map(|segment| identifier_name(&segment.ident))
                        .unwrap_or_else(|| "<empty-path>".to_owned());
                    if invocation != "compile_error" {
                        violations.push(format!(
                            "{label}: ancestor macro invocation {invocation}! can introduce shadowing candidate-test macros"
                        ));
                    }
                }
            }
            Item::Use(item_use) => {
                let mut bindings = BTreeSet::new();
                let mut contains_glob = false;
                collect_use_tree_candidate_macro_bindings(
                    &item_use.tree,
                    None,
                    &mut bindings,
                    &mut contains_glob,
                );
                for binding in bindings {
                    violations.push(format!(
                        "{label}: ancestor scope can shadow candidate-test macro {binding}! with an import or alias"
                    ));
                }
                if contains_glob {
                    violations.push(format!(
                        "{label}: ancestor glob import can shadow candidate-test macros"
                    ));
                }
            }
            Item::ExternCrate(item_extern_crate) => {
                let local = item_extern_crate
                    .rename
                    .as_ref()
                    .map(|(_, identifier)| identifier)
                    .unwrap_or(&item_extern_crate.ident);
                let name = identifier_name(local);
                if is_candidate_test_ancestor_guard_macro(&name) {
                    violations.push(format!(
                        "{label}: ancestor extern-crate alias can shadow candidate-test macro {name}!"
                    ));
                }
            }
            _ => {}
        }
    }
}

fn check_candidate_test_module_export_hazards(
    module: &ItemMod,
    label: &str,
    violations: &mut Vec<String>,
) {
    if module
        .attrs
        .iter()
        .any(|attribute| path_is_identifier(attribute.path(), "macro_use"))
    {
        violations.push(format!(
            "{label}: nested module {} uses #[macro_use] and can shadow candidate-test macros",
            module.ident
        ));
    }
    let Some((_, items)) = &module.content else {
        return;
    };
    for item in items {
        match item {
            Item::Macro(item_macro) => {
                if item_macro
                    .attrs
                    .iter()
                    .any(|attribute| path_is_identifier(attribute.path(), "macro_export"))
                {
                    violations.push(format!(
                        "{label}: nested module {} exports a macro that can shadow candidate-test macros",
                        module.ident
                    ));
                }
                if item_macro.ident.is_none() {
                    violations.push(format!(
                        "{label}: nested module {} invokes an item macro that can export candidate-test macro bindings",
                        module.ident
                    ));
                }
            }
            Item::Mod(nested) => {
                check_candidate_test_module_export_hazards(nested, label, violations);
            }
            Item::ExternCrate(item_extern_crate)
                if item_extern_crate
                    .attrs
                    .iter()
                    .any(|attribute| path_is_identifier(attribute.path(), "macro_use")) =>
            {
                violations.push(format!(
                    "{label}: nested module {} imports macros that can shadow candidate-test macros",
                    module.ident
                ));
            }
            _ => {}
        }
    }
}

fn collect_use_tree_candidate_macro_bindings(
    tree: &UseTree,
    parent: Option<&syn::Ident>,
    bindings: &mut BTreeSet<String>,
    contains_glob: &mut bool,
) {
    match tree {
        UseTree::Path(path) => collect_use_tree_candidate_macro_bindings(
            &path.tree,
            Some(&path.ident),
            bindings,
            contains_glob,
        ),
        UseTree::Name(name) => {
            let local = if identifier_is(&name.ident, "self") {
                parent
            } else {
                Some(&name.ident)
            };
            if let Some(local) = local {
                let local = identifier_name(local);
                if is_candidate_test_ancestor_guard_macro(&local) {
                    bindings.insert(local);
                }
            }
        }
        UseTree::Rename(rename) => {
            let local = identifier_name(&rename.rename);
            if is_candidate_test_ancestor_guard_macro(&local) {
                bindings.insert(local);
            }
        }
        UseTree::Glob(_) => *contains_glob = true,
        UseTree::Group(group) => {
            for nested in &group.items {
                collect_use_tree_candidate_macro_bindings(nested, parent, bindings, contains_glob);
            }
        }
    }
}

fn use_tree_mentions_identifier(tree: &UseTree, expected: &str) -> bool {
    match tree {
        UseTree::Path(path) => {
            identifier_is(&path.ident, expected)
                || use_tree_mentions_identifier(&path.tree, expected)
        }
        UseTree::Name(name) => identifier_is(&name.ident, expected),
        UseTree::Rename(rename) => identifier_is(&rename.ident, expected),
        UseTree::Glob(_) => false,
        UseTree::Group(group) => group
            .items
            .iter()
            .any(|item| use_tree_mentions_identifier(item, expected)),
    }
}

fn is_candidate_test_allowed_macro(name: &str) -> bool {
    CANDIDATE_TEST_ALLOWED_MACROS.contains(&name)
}

fn is_candidate_test_ancestor_guard_macro(name: &str) -> bool {
    is_candidate_test_allowed_macro(name) || name == "compile_error"
}

#[derive(Default)]
struct CandidateTestCollector<'ast> {
    tests: Vec<CandidateTest<'ast>>,
    cfg_test_ancestor_count: usize,
    unsupported_conditional_ancestor_count: usize,
}

struct CandidateTest<'ast> {
    item: &'ast syn::ItemFn,
    cfg_test_ancestor_count: usize,
    unsupported_conditional_ancestor_count: usize,
}

impl<'ast> CandidateTestCollector<'ast> {
    fn visit_candidate_item(&mut self, item: &'ast Item) {
        match item {
            Item::Fn(function) => self.visit_item_fn(function),
            Item::Mod(module) => self.visit_item_mod(module),
            _ => {}
        }
    }

    fn enter_ancestor_attributes(&mut self, attributes: &[Attribute]) {
        for attribute in attributes {
            if exact_cfg_test_attribute(attribute) {
                self.cfg_test_ancestor_count += 1;
            } else if path_is_identifier(attribute.path(), "cfg")
                || path_is_identifier(attribute.path(), "cfg_attr")
            {
                self.unsupported_conditional_ancestor_count += 1;
            }
        }
    }

    fn restore_ancestor_counts(&mut self, cfg_test: usize, unsupported: usize) {
        self.cfg_test_ancestor_count = cfg_test;
        self.unsupported_conditional_ancestor_count = unsupported;
    }
}

impl<'ast> Visit<'ast> for CandidateTestCollector<'ast> {
    fn visit_file(&mut self, file: &'ast syn::File) {
        let previous_cfg_test = self.cfg_test_ancestor_count;
        let previous_unsupported = self.unsupported_conditional_ancestor_count;
        self.enter_ancestor_attributes(&file.attrs);
        for item in &file.items {
            self.visit_candidate_item(item);
        }
        self.restore_ancestor_counts(previous_cfg_test, previous_unsupported);
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let previous_cfg_test = self.cfg_test_ancestor_count;
        let previous_unsupported = self.unsupported_conditional_ancestor_count;
        self.enter_ancestor_attributes(&item.attrs);
        if let Some((_, items)) = &item.content {
            for nested in items {
                self.visit_candidate_item(nested);
            }
        }
        self.restore_ancestor_counts(previous_cfg_test, previous_unsupported);
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if item
            .attrs
            .iter()
            .any(|attribute| path_is_identifier(attribute.path(), "test"))
        {
            self.tests.push(CandidateTest {
                item,
                cfg_test_ancestor_count: self.cfg_test_ancestor_count,
                unsupported_conditional_ancestor_count: self.unsupported_conditional_ancestor_count,
            });
        }
    }
}

fn exact_cfg_test_attribute(attribute: &Attribute) -> bool {
    if !path_is_identifier(attribute.path(), "cfg") {
        return false;
    }
    let Meta::List(list) = &attribute.meta else {
        return false;
    };
    let Some(arguments) = parse_meta_arguments(list) else {
        return false;
    };
    arguments.len() == 1
        && matches!(
            arguments.first(),
            Some(Meta::Path(path)) if path_is_identifier(path, "test")
        )
}

fn check_candidate_test(test: &CandidateTest<'_>, label: &str, violations: &mut Vec<String>) {
    let item = test.item;
    let name = identifier_name(&item.sig.ident);
    if test.cfg_test_ancestor_count != 1 {
        violations.push(format!(
            "{label}: test {name} must be nested under exactly one #[cfg(test)] ancestor"
        ));
    }
    if test.unsupported_conditional_ancestor_count != 0 {
        violations.push(format!(
            "{label}: test {name} has an unsupported ancestor conditional attribute"
        ));
    }
    let test_attribute_count = item
        .attrs
        .iter()
        .filter(|attribute| path_is_identifier(attribute.path(), "test"))
        .count();
    if test_attribute_count != 1 {
        violations.push(format!(
            "{label}: test {name} must have exactly one #[test] attribute"
        ));
    }
    for attribute in &item.attrs {
        match prohibited_candidate_test_attribute(&attribute.meta) {
            Ok(Some("ignore")) => {
                violations.push(format!("{label}: ignored test {name} is forbidden"))
            }
            Ok(Some("should_panic")) => {
                violations.push(format!("{label}: should-panic test {name} is forbidden"))
            }
            Ok(Some(_)) | Ok(None) => {}
            Err(()) => violations.push(format!("{label}: test {name} has an unparseable cfg_attr")),
        }
    }
    if candidate_test_platform(&item.attrs).is_err() {
        violations.push(format!(
            "{label}: test {name} has an unsupported conditional attribute"
        ));
    }
    if !item.sig.inputs.is_empty()
        || !matches!(item.sig.output, syn::ReturnType::Default)
        || item.sig.asyncness.is_some()
        || item.sig.constness.is_some()
        || item.sig.unsafety.is_some()
        || item.sig.abi.is_some()
        || item.sig.variadic.is_some()
        || !item.sig.generics.params.is_empty()
        || item.sig.generics.where_clause.is_some()
    {
        violations.push(format!(
            "{label}: test {name} must be a plain zero-argument unit test"
        ));
    }
    let mut return_visitor = CandidateTestReturnVisitor::default();
    return_visitor.visit_block(&item.block);
    if return_visitor.found {
        violations.push(format!(
            "{label}: test {name} contains an early-success return"
        ));
    }
}

fn check_required_candidate_test_body(
    test: &CandidateTest<'_>,
    label: &str,
    violations: &mut Vec<String>,
) {
    let name = identifier_name(&test.item.sig.ident);
    if test.item.block.stmts.is_empty() {
        violations.push(format!("{label}: required test {name} has an empty body"));
        return;
    }
    let mut evidence = CandidateTestEvidenceVisitor::default();
    evidence.visit_block(&test.item.block);
    if evidence.assertions == 0 {
        violations.push(format!(
            "{label}: required test {name} must contain an explicit assertion"
        ));
    }
    if evidence.actions == 0 {
        violations.push(format!(
            "{label}: required test {name} must exercise at least one function or method call outside macro expansion"
        ));
    }
    if evidence.unsupported_conditionals != 0 {
        violations.push(format!(
            "{label}: required test {name} contains an unsupported body conditional attribute"
        ));
    }
    for unsupported in evidence.unsupported_macros {
        violations.push(format!(
            "{label}: required test {name} invokes unsupported macro {unsupported}!"
        ));
    }
}

#[derive(Default)]
struct CandidateTestEvidenceVisitor {
    assertions: usize,
    actions: usize,
    unsupported_conditionals: usize,
    unsupported_macros: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for CandidateTestEvidenceVisitor {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if path_is_identifier(attribute.path(), "cfg_attr")
            || path_is_identifier(attribute.path(), "cfg")
        {
            self.unsupported_conditionals += 1;
        }
        syn::visit::visit_attribute(self, attribute);
    }

    fn visit_expr_call(&mut self, expression: &'ast syn::ExprCall) {
        self.actions += 1;
        syn::visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        self.actions += 1;
        syn::visit::visit_expr_method_call(self, expression);
    }

    fn visit_macro(&mut self, expression: &'ast syn::Macro) {
        let Some(segment) = expression.path.segments.last() else {
            self.unsupported_macros.insert("<empty-path>".to_owned());
            return;
        };
        let name = identifier_name(&segment.ident);
        if matches!(name.as_str(), "assert" | "assert_eq" | "assert_ne") {
            self.assertions += 1;
            if let Ok(arguments) = Punctuated::<syn::Expr, Token![,]>::parse_terminated
                .parse2(expression.tokens.clone())
            {
                for argument in &arguments {
                    self.visit_expr(argument);
                }
            }
        }
        if !is_candidate_test_allowed_macro(&name) {
            self.unsupported_macros.insert(name);
        }
    }

    fn visit_item_fn(&mut self, _item: &'ast syn::ItemFn) {}
}

fn prohibited_candidate_test_attribute(meta: &Meta) -> Result<Option<&'static str>, ()> {
    if path_is_identifier(meta.path(), "ignore") {
        return Ok(Some("ignore"));
    }
    if path_is_identifier(meta.path(), "should_panic") {
        return Ok(Some("should_panic"));
    }
    let Meta::List(list) = meta else {
        return Ok(None);
    };
    if !path_is_identifier(&list.path, "cfg_attr") {
        return Ok(None);
    }
    let nested = parse_meta_arguments(list).ok_or(())?;
    for meta in nested.iter().skip(1) {
        if let Some(prohibited) = prohibited_candidate_test_attribute(meta)? {
            return Ok(Some(prohibited));
        }
    }
    Ok(None)
}

fn candidate_test_platform(attributes: &[Attribute]) -> Result<Option<CandidateTestPlatform>, ()> {
    let mut platform = None;
    for attribute in attributes {
        if path_is_identifier(attribute.path(), "cfg_attr") {
            return Err(());
        }
        if !path_is_identifier(attribute.path(), "cfg") {
            continue;
        }
        let Meta::List(list) = &attribute.meta else {
            return Err(());
        };
        let arguments = parse_meta_arguments(list).ok_or(())?;
        if arguments.len() != 1 {
            return Err(());
        }
        let meta = arguments.first().ok_or(())?;
        let next = if path_is_identifier(meta.path(), "windows") {
            CandidateTestPlatform::Windows
        } else if path_is_identifier(meta.path(), "unix") {
            CandidateTestPlatform::Unix
        } else {
            return Err(());
        };
        if platform.replace(next).is_some() {
            return Err(());
        }
    }
    Ok(platform)
}

#[derive(Default)]
struct CandidateTestReturnVisitor {
    found: bool,
}

impl<'ast> Visit<'ast> for CandidateTestReturnVisitor {
    fn visit_expr_return(&mut self, _expression: &'ast syn::ExprReturn) {
        self.found = true;
    }

    fn visit_expr_closure(&mut self, _expression: &'ast syn::ExprClosure) {}

    fn visit_expr_async(&mut self, _expression: &'ast syn::ExprAsync) {}

    fn visit_item_fn(&mut self, _item: &'ast syn::ItemFn) {}
}

/// Scans one Rust source document for direct production access to `std::fs`.
#[doc(hidden)]
pub fn production_source_text_violations(source: &str) -> Vec<String> {
    const SOURCE_LABEL: &str = "inline production source";

    let file = match syn::parse_file(source) {
        Ok(file) => file,
        Err(error) => {
            return vec![format!(
                "native-fs source policy: cannot parse {SOURCE_LABEL}: {error}"
            )];
        }
    };
    let mut violations = Vec::new();
    scan_inline_production_items(
        SOURCE_LABEL,
        &file.items,
        &default_std_aliases(),
        &BTreeSet::new(),
        &mut violations,
    );
    violations.sort_unstable();
    violations.dedup();
    violations
}

/// Rejects evidence-only conditional compilation in candidate production code.
#[doc(hidden)]
pub fn candidate_production_source_violations(source: &str) -> Vec<String> {
    const SOURCE_LABEL: &str = "inline candidate production source";

    let file = match syn::parse_file(source) {
        Ok(file) => file,
        Err(error) => {
            return vec![format!(
                "native-fs candidate production source: cannot parse {SOURCE_LABEL}: {error}"
            )];
        }
    };
    let mut violations = Vec::new();
    check_candidate_implementation_cfg_file(&file, SOURCE_LABEL, &mut violations);
    violations.sort_unstable();
    violations.dedup();
    violations
}

fn check_candidate_implementation_cfg_sources(
    repository_root: &Path,
    violations: &mut Vec<String>,
) {
    let mut visited = BTreeSet::new();
    for relative_source in [
        CANDIDATE_TEST_SOURCE_PATH,
        REPOSITORY_CANDIDATE_TEST_SOURCE_PATH,
    ] {
        let source = repository_root.join(relative_source);
        let module_directory = module_directory_for_file(&source);
        scan_candidate_implementation_cfg_file(
            repository_root,
            &source,
            &module_directory,
            &mut visited,
            violations,
        );
    }
}

fn scan_candidate_implementation_cfg_file(
    repository_root: &Path,
    source: &Path,
    module_directory: &Path,
    visited: &mut BTreeSet<PathBuf>,
    violations: &mut Vec<String>,
) {
    let canonical = match fs::canonicalize(source) {
        Ok(path) => path,
        Err(error) => {
            violations.push(format!(
                "native-fs candidate production source: cannot inspect {}: {error}",
                display_source_path(repository_root, source)
            ));
            return;
        }
    };
    if !visited.insert(canonical) {
        return;
    }
    let source_text = match fs::read_to_string(source) {
        Ok(source_text) => source_text,
        Err(error) => {
            violations.push(format!(
                "native-fs candidate production source: cannot read {}: {error}",
                display_source_path(repository_root, source)
            ));
            return;
        }
    };
    let file = match syn::parse_file(&source_text) {
        Ok(file) => file,
        Err(error) => {
            violations.push(format!(
                "native-fs candidate production source: cannot parse {}: {error}",
                display_source_path(repository_root, source)
            ));
            return;
        }
    };
    let source_label = display_source_path(repository_root, source);
    check_candidate_implementation_cfg_file(&file, &source_label, violations);
    scan_candidate_implementation_cfg_modules(
        repository_root,
        &source_label,
        module_directory,
        &file.items,
        visited,
        violations,
    );
}

fn scan_candidate_implementation_cfg_modules(
    repository_root: &Path,
    source_label: &str,
    module_directory: &Path,
    items: &[Item],
    visited: &mut BTreeSet<PathBuf>,
    violations: &mut Vec<String>,
) {
    for item in items {
        let Item::Mod(module) = item else {
            continue;
        };
        if exact_candidate_test_module(module) {
            continue;
        }
        if let Some(attribute) = module_source_override(&module.attrs) {
            violations.push(format!(
                "native-fs candidate production source: module {} in {source_label} uses forbidden {attribute} source redirection",
                module.ident
            ));
            continue;
        }
        if let Some((_, nested_items)) = &module.content {
            scan_candidate_implementation_cfg_modules(
                repository_root,
                source_label,
                &module_directory.join(identifier_name(&module.ident)),
                nested_items,
                visited,
                violations,
            );
            continue;
        }
        let Some(module_file) = resolve_module_file(module_directory, module) else {
            violations.push(format!(
                "native-fs candidate production source: cannot resolve module {} from {source_label}",
                module.ident
            ));
            continue;
        };
        let nested_directory = module_directory_for_file(&module_file);
        scan_candidate_implementation_cfg_file(
            repository_root,
            &module_file,
            &nested_directory,
            visited,
            violations,
        );
    }
}

fn check_candidate_implementation_cfg_file(
    file: &syn::File,
    source_label: &str,
    violations: &mut Vec<String>,
) {
    let mut visitor = CandidateImplementationCfgVisitor {
        source_label,
        violations,
    };
    visitor.visit_file(file);
}

fn exact_candidate_test_module(module: &ItemMod) -> bool {
    identifier_is(&module.ident, "tests")
        && module.attrs.len() == 1
        && exact_cfg_test_attribute(&module.attrs[0])
        && matches!(module.vis, Visibility::Inherited)
}

struct CandidateImplementationCfgVisitor<'a> {
    source_label: &'a str,
    violations: &'a mut Vec<String>,
}

impl<'ast> Visit<'ast> for CandidateImplementationCfgVisitor<'_> {
    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if exact_candidate_test_module(module) {
            return;
        }
        syn::visit::visit_item_mod(self, module);
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if path_is_identifier(&item.mac.path, "macro_rules") {
            self.violations.push(format!(
                "native-fs candidate production source: macro_rules! is forbidden in {} because it can hide evidence-only cfg",
                self.source_label
            ));
        }
        syn::visit::visit_item_macro(self, item);
    }

    fn visit_macro(&mut self, expression: &'ast syn::Macro) {
        if path_ends_in_identifier(&expression.path, "cfg") {
            self.violations.push(format!(
                "native-fs candidate production source: cfg! is forbidden in {}",
                self.source_label
            ));
        } else if path_ends_in_identifier(&expression.path, "include") {
            self.violations.push(format!(
                "native-fs candidate production source: include! is forbidden in {}",
                self.source_label
            ));
        } else if !is_admitted_candidate_implementation_macro(&expression.path) {
            let name = expression.path.segments.last().map_or_else(
                || "<empty-path>".to_owned(),
                |segment| identifier_name(&segment.ident),
            );
            self.violations.push(format!(
                "native-fs candidate production source: macro {name}! is not an admitted exact core builtin in {}",
                self.source_label
            ));
        }
        syn::visit::visit_macro(self, expression);
    }

    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if path_is_identifier(attribute.path(), "cfg_attr") {
            self.violations.push(format!(
                "native-fs candidate production source: cfg_attr is forbidden in {}",
                self.source_label
            ));
        } else if path_is_identifier(attribute.path(), "cfg") {
            match candidate_implementation_evidence_cfg(attribute) {
                Ok(Some(condition)) => self.violations.push(format!(
                    "native-fs candidate production source: evidence-only cfg condition {condition} is forbidden in {}",
                    self.source_label
                )),
                Ok(None) => {}
                Err(()) => self.violations.push(format!(
                    "native-fs candidate production source: unparseable cfg attribute is forbidden in {}",
                    self.source_label
                )),
            }
        }
        syn::visit::visit_attribute(self, attribute);
    }
}

fn candidate_implementation_evidence_cfg(
    attribute: &Attribute,
) -> Result<Option<&'static str>, ()> {
    let Meta::List(list) = &attribute.meta else {
        return Err(());
    };
    let arguments = parse_meta_arguments(list).ok_or(())?;
    for argument in &arguments {
        if let Some(condition) = evidence_only_cfg_meta(argument)? {
            return Ok(Some(condition));
        }
    }
    Ok(None)
}

fn evidence_only_cfg_meta(meta: &Meta) -> Result<Option<&'static str>, ()> {
    match meta {
        Meta::Path(path) if path_is_identifier(path, "test") => Ok(Some("test")),
        Meta::Path(path) if path_is_identifier(path, "debug_assertions") => {
            Ok(Some("debug_assertions"))
        }
        Meta::Path(path) if path_is_identifier(path, "miri") => Ok(Some("miri")),
        Meta::Path(_) | Meta::NameValue(_) => Ok(None),
        Meta::List(list) => {
            let arguments = parse_meta_arguments(list).ok_or(())?;
            for argument in &arguments {
                if let Some(condition) = evidence_only_cfg_meta(argument)? {
                    return Ok(Some(condition));
                }
            }
            Ok(None)
        }
    }
}

/// Loads Cargo metadata and checks the production parser source graph.
///
/// This is public only so adversarial workspace fixtures can exercise the same
/// metadata boundary as the repository check.
#[doc(hidden)]
pub fn repository_production_source_violations(repository_root: &Path) -> Vec<String> {
    let metadata = match load_metadata(
        repository_root,
        FACADE_MANIFEST_PATH,
        None,
        Some(FACADE_FORMAT_FEATURE),
        false,
    ) {
        Ok(metadata) => metadata,
        Err(violation) => return vec![violation],
    };
    let mut violations = Vec::new();
    check_format_import_production_sources(repository_root, &metadata, &mut violations);
    violations.sort_unstable();
    violations.dedup();
    violations
}

fn check_format_import_production_sources(
    repository_root: &Path,
    metadata: &Value,
    violations: &mut Vec<String>,
) {
    let packages = match facade_parser_packages(metadata) {
        Ok(packages) => packages,
        Err(violation) => {
            violations.push(violation);
            return;
        }
    };
    let mut roots = Vec::new();
    for (name, package) in packages {
        let Some(targets) = package.get("targets").and_then(Value::as_array) else {
            violations.push(format!(
                "native-fs source policy: cargo metadata omitted targets for {name}"
            ));
            continue;
        };
        for target in targets {
            let Some(kinds) = target.get("kind").and_then(Value::as_array) else {
                violations.push(format!(
                    "native-fs source policy: cargo metadata omitted target kind for {name}"
                ));
                continue;
            };
            if !kinds.iter().any(is_production_target_kind) {
                continue;
            }
            let Some(source) = string_field(target, "src_path") else {
                violations.push(format!(
                    "native-fs source policy: cargo metadata omitted production source for {name}"
                ));
                continue;
            };
            roots.push((name.to_owned(), PathBuf::from(source)));
        }
    }

    roots.sort_unstable();
    roots.dedup();
    let mut visited = BTreeSet::new();
    for (package, source) in roots {
        let module_directory = source.parent().unwrap_or(repository_root).to_path_buf();
        scan_format_import_source_file(
            repository_root,
            &package,
            &source,
            &module_directory,
            &default_std_aliases(),
            &BTreeSet::new(),
            &mut visited,
            violations,
        );
    }
}

fn facade_parser_packages(metadata: &Value) -> Result<Vec<(&str, &Value)>, String> {
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "native-fs source policy: cargo metadata omitted packages".to_owned())?;
    let mut packages_by_id = BTreeMap::new();
    let mut facade_ids = Vec::new();
    for package in packages {
        let id = string_field(package, "id").ok_or_else(|| {
            "native-fs source policy: cargo metadata contained a package without an id".to_owned()
        })?;
        let name = string_field(package, "name").ok_or_else(|| {
            "native-fs source policy: cargo metadata contained a package without a name".to_owned()
        })?;
        if packages_by_id.insert(id, package).is_some() {
            return Err(format!(
                "native-fs source policy: cargo metadata repeated package id {id}"
            ));
        }
        if name == FACADE_PACKAGE {
            facade_ids.push(id);
        }
    }
    let [facade_id] = facade_ids.as_slice() else {
        return Err(format!(
            "native-fs source policy: expected exactly one {FACADE_PACKAGE} facade package, found {}",
            facade_ids.len()
        ));
    };

    let nodes = metadata
        .get("resolve")
        .and_then(|resolve| resolve.get("nodes"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "native-fs source policy: cargo metadata omitted the complete resolve graph".to_owned()
        })?;
    let mut production_edges = BTreeMap::new();
    for node in nodes {
        let id = string_field(node, "id").ok_or_else(|| {
            "native-fs source policy: resolved package node omitted its id".to_owned()
        })?;
        let dependencies = node.get("deps").and_then(Value::as_array).ok_or_else(|| {
            format!("native-fs source policy: resolved package {id} omitted dependencies")
        })?;
        let mut edges = Vec::new();
        for dependency in dependencies {
            if resolved_dependency_is_production(dependency)? {
                let package = string_field(dependency, "pkg").ok_or_else(|| {
                    format!(
                        "native-fs source policy: resolved dependency of {id} omitted its package id"
                    )
                })?;
                edges.push(package);
            }
        }
        edges.sort_unstable();
        edges.dedup();
        if production_edges.insert(id, edges).is_some() {
            return Err(format!(
                "native-fs source policy: cargo metadata repeated resolve node {id}"
            ));
        }
    }

    let mut reachable = BTreeSet::from([*facade_id]);
    let mut pending = VecDeque::from([*facade_id]);
    while let Some(id) = pending.pop_front() {
        let dependencies = production_edges.get(id).ok_or_else(|| {
            format!("native-fs source policy: production package {id} omitted its resolve node")
        })?;
        for dependency in dependencies {
            if !packages_by_id.contains_key(dependency) {
                return Err(format!(
                    "native-fs source policy: production dependency {dependency} omitted its package metadata"
                ));
            }
            if reachable.insert(dependency) {
                pending.push_back(dependency);
            }
        }
    }

    let mut selected = Vec::new();
    let mut selected_names = BTreeMap::new();
    let mut import_count = 0_usize;
    let mut format_count = 0_usize;
    for id in reachable {
        let package = packages_by_id.get(id).ok_or_else(|| {
            format!("native-fs source policy: reachable package {id} omitted metadata")
        })?;
        let name = string_field(package, "name").ok_or_else(|| {
            format!("native-fs source policy: reachable package {id} omitted its name")
        })?;
        if name != IMPORT_PACKAGE && !name.starts_with(FORMAT_PACKAGE_PREFIX) {
            continue;
        }
        if let Some(previous_id) = selected_names.insert(name, id) {
            return Err(format!(
                "native-fs source policy: facade production parser set contains duplicate package name {name}: {previous_id} and {id}"
            ));
        }
        if name == IMPORT_PACKAGE {
            import_count = import_count.saturating_add(1);
        } else {
            format_count = format_count.saturating_add(1);
        }
        selected.push((name, *package));
    }
    if import_count != 1 {
        return Err(format!(
            "native-fs source policy: facade production parser set must contain exactly one {IMPORT_PACKAGE}, found {import_count}"
        ));
    }
    if format_count == 0 {
        return Err(format!(
            "native-fs source policy: facade production parser set contains no {FORMAT_PACKAGE_PREFIX}* package"
        ));
    }
    selected.sort_unstable_by_key(|(name, _)| *name);
    Ok(selected)
}

fn resolved_dependency_is_production(dependency: &Value) -> Result<bool, String> {
    let kinds = dependency
        .get("dep_kinds")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "native-fs source policy: resolved dependency omitted dep_kinds".to_owned()
        })?;
    let mut production = false;
    for kind in kinds {
        match kind.get("kind") {
            Some(Value::Null) => production = true,
            Some(Value::String(value)) if value == "build" => production = true,
            Some(Value::String(value)) if value == "dev" => {}
            Some(Value::String(value)) => {
                return Err(format!(
                    "native-fs source policy: resolved dependency has unknown kind {value}"
                ));
            }
            _ => {
                return Err(
                    "native-fs source policy: resolved dependency kind is not null or a string"
                        .to_owned(),
                );
            }
        }
    }
    Ok(production)
}

fn is_production_target_kind(value: &Value) -> bool {
    value.as_str().is_some_and(|kind| {
        matches!(
            kind,
            "bin"
                | "cdylib"
                | "custom-build"
                | "dylib"
                | "lib"
                | "proc-macro"
                | "rlib"
                | "staticlib"
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn scan_format_import_source_file(
    repository_root: &Path,
    package: &str,
    source: &Path,
    module_directory: &Path,
    inherited_aliases: &BTreeSet<String>,
    inherited_macros: &BTreeSet<String>,
    visited: &mut BTreeSet<PathBuf>,
    violations: &mut Vec<String>,
) {
    let canonical = match fs::canonicalize(source) {
        Ok(path) => path,
        Err(error) => {
            violations.push(format!(
                "native-fs source policy: cannot inspect production source {} ({package}): {error}",
                display_source_path(repository_root, source)
            ));
            return;
        }
    };
    if !visited.insert(canonical) {
        return;
    }

    let source_text = match fs::read_to_string(source) {
        Ok(source_text) => source_text,
        Err(error) => {
            violations.push(format!(
                "native-fs source policy: cannot read production source {} ({package}): {error}",
                display_source_path(repository_root, source)
            ));
            return;
        }
    };
    let file = match syn::parse_file(&source_text) {
        Ok(file) => file,
        Err(error) => {
            violations.push(format!(
                "native-fs source policy: cannot parse production source {} ({package}): {error}",
                display_source_path(repository_root, source)
            ));
            return;
        }
    };
    scan_repository_module_items(
        repository_root,
        package,
        source,
        module_directory,
        &file.items,
        inherited_aliases,
        inherited_macros,
        visited,
        violations,
    );
}

#[allow(clippy::too_many_arguments)]
fn scan_repository_module_items(
    repository_root: &Path,
    package: &str,
    source: &Path,
    module_directory: &Path,
    items: &[Item],
    inherited_aliases: &BTreeSet<String>,
    inherited_macros: &BTreeSet<String>,
    visited: &mut BTreeSet<PathBuf>,
    violations: &mut Vec<String>,
) {
    let aliases = std_aliases_from_items(items, inherited_aliases);
    let macros = safe_declarative_macros_from_items(items, inherited_macros);
    let source_label = display_source_path(repository_root, source);
    for item in items {
        if item_is_test_only(item) {
            continue;
        }
        if let Item::Mod(item_mod) = item {
            if let Some(attribute) = module_source_override(&item_mod.attrs) {
                violations.push(format!(
                    "native-fs source policy: production module {} in {source_label} ({package}) uses forbidden {attribute} source redirection",
                    item_mod.ident
                ));
                continue;
            }
            if let Some((_, nested_items)) = &item_mod.content {
                scan_repository_module_items(
                    repository_root,
                    package,
                    source,
                    &module_directory.join(identifier_name(&item_mod.ident)),
                    nested_items,
                    &aliases,
                    &macros,
                    visited,
                    violations,
                );
            } else if let Some(module_file) = resolve_module_file(module_directory, item_mod) {
                let nested_directory = module_directory_for_file(&module_file);
                scan_format_import_source_file(
                    repository_root,
                    package,
                    &module_file,
                    &nested_directory,
                    &aliases,
                    &macros,
                    visited,
                    violations,
                );
            } else {
                violations.push(format!(
                    "native-fs source policy: cannot resolve production module {} from {source_label} ({package})",
                    item_mod.ident
                ));
            }
            continue;
        }
        inspect_production_item(&source_label, package, item, &aliases, &macros, violations);
    }
}

fn scan_inline_production_items(
    source_label: &str,
    items: &[Item],
    inherited_aliases: &BTreeSet<String>,
    inherited_macros: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    let aliases = std_aliases_from_items(items, inherited_aliases);
    let macros = safe_declarative_macros_from_items(items, inherited_macros);
    for item in items {
        if item_is_test_only(item) {
            continue;
        }
        if let Item::Mod(item_mod) = item {
            if let Some(attribute) = module_source_override(&item_mod.attrs) {
                violations.push(format!(
                    "native-fs source policy: production module {} in {source_label} (fixture) uses forbidden {attribute} source redirection",
                    item_mod.ident
                ));
                continue;
            }
            if let Some((_, nested_items)) = &item_mod.content {
                scan_inline_production_items(
                    source_label,
                    nested_items,
                    &aliases,
                    &macros,
                    violations,
                );
            } else {
                violations.push(format!(
                    "native-fs source policy: out-of-line production module {} in {source_label} (fixture) has no verified source traversal",
                    item_mod.ident
                ));
            }
            continue;
        }
        inspect_production_item(source_label, "fixture", item, &aliases, &macros, violations);
    }
}

fn inspect_production_item(
    source_label: &str,
    package: &str,
    item: &Item,
    aliases: &BTreeSet<String>,
    safe_macros: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    if let Item::Use(item_use) = item
        && use_tree_reaches_std_fs(&item_use.tree, &mut Vec::new(), aliases)
    {
        violations.push(format!(
            "native-fs source policy: direct host filesystem import in {source_label} ({package}) bypasses baozi-io"
        ));
    }

    let mut visitor = ProductionFilesystemVisitor {
        aliases: aliases.clone(),
        safe_macros: safe_macros.clone(),
        source_label,
        package,
        violations,
    };
    visitor.visit_item(item);
}

fn default_std_aliases() -> BTreeSet<String> {
    BTreeSet::from(["std".to_owned()])
}

fn safe_declarative_macros_from_items(
    items: &[Item],
    inherited_macros: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut macros = inherited_macros.clone();
    for item in items {
        extend_safe_declarative_macro(item, &mut macros);
    }
    macros
}

fn extend_safe_declarative_macro(item: &Item, macros: &mut BTreeSet<String>) {
    if item_is_test_only(item) {
        return;
    }
    let Item::Macro(item_macro) = item else {
        return;
    };
    let Some(name) = &item_macro.ident else {
        return;
    };
    let name = identifier_name(name);
    macros.remove(&name);
    if !path_is_identifier(&item_macro.mac.path, "macro_rules")
        || !macro_tokens_are_source_closed(&item_macro.mac.tokens)
    {
        return;
    }
    macros.insert(name);
}

fn macro_source_hazard(tokens: &TokenStream) -> Option<&'static str> {
    macro_token_hazard(tokens, true)
}

fn macro_expression_hazard(tokens: &TokenStream) -> Option<&'static str> {
    macro_token_hazard(tokens, false)
}

fn macro_token_hazard(tokens: &TokenStream, reject_standard_library: bool) -> Option<&'static str> {
    for token in tokens.clone() {
        match token {
            TokenTree::Group(group) => {
                if let Some(hazard) = macro_token_hazard(&group.stream(), reject_standard_library) {
                    return Some(hazard);
                }
            }
            TokenTree::Ident(identifier) => match token_identifier(&identifier.to_string()) {
                "std" if reject_standard_library => {
                    return Some("standard-library identifier inside a macro");
                }
                "fs" => return Some("filesystem identifier inside a macro"),
                "include" => return Some("Rust source include inside a macro"),
                _ => {}
            },
            TokenTree::Literal(_) | TokenTree::Punct(_) => {}
        }
    }
    None
}

fn macro_tokens_are_source_closed(tokens: &TokenStream) -> bool {
    macro_source_hazard(tokens).is_none()
        && !tokens.clone().into_iter().any(|token| match token {
            TokenTree::Group(group) => !macro_tokens_are_source_closed(&group.stream()),
            TokenTree::Punct(punctuation) => punctuation.as_char() == '!',
            TokenTree::Ident(_) | TokenTree::Literal(_) => false,
        })
}

fn simple_macro_name(item: &syn::Macro) -> Option<String> {
    if item.path.segments.len() != 1 {
        return None;
    }
    item.path
        .segments
        .first()
        .map(|segment| identifier_name(&segment.ident))
}

fn std_aliases_from_items(
    items: &[Item],
    inherited_aliases: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut aliases = inherited_aliases.clone();
    aliases.insert("std".to_owned());
    for item in items {
        if item_is_test_only(item) {
            continue;
        }
        if let Item::ExternCrate(item_extern) = item
            && identifier_is(&item_extern.ident, "std")
        {
            aliases.insert(
                item_extern
                    .rename
                    .as_ref()
                    .map_or_else(|| "std".to_owned(), |(_, alias)| identifier_name(alias)),
            );
        }
    }

    loop {
        let known_aliases = aliases.clone();
        let mut discovered = BTreeSet::new();
        for item in items {
            if item_is_test_only(item) {
                continue;
            }
            if let Item::Use(item_use) = item {
                collect_std_aliases_from_use_tree(
                    &item_use.tree,
                    &mut Vec::new(),
                    &known_aliases,
                    &mut discovered,
                );
            }
        }
        let previous_len = aliases.len();
        aliases.extend(discovered);
        if aliases.len() == previous_len {
            return aliases;
        }
    }
}

fn collect_std_aliases_from_use_tree(
    tree: &UseTree,
    prefix: &mut Vec<String>,
    known_aliases: &BTreeSet<String>,
    discovered: &mut BTreeSet<String>,
) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(identifier_name(&path.ident));
            collect_std_aliases_from_use_tree(&path.tree, prefix, known_aliases, discovered);
            prefix.pop();
        }
        UseTree::Name(name) => {
            let mut path = prefix.clone();
            path.push(identifier_name(&name.ident));
            register_std_alias(
                &path,
                identifier_name(&name.ident),
                known_aliases,
                discovered,
            );
        }
        UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            path.push(identifier_name(&rename.ident));
            register_std_alias(
                &path,
                identifier_name(&rename.rename),
                known_aliases,
                discovered,
            );
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                collect_std_aliases_from_use_tree(tree, prefix, known_aliases, discovered);
            }
        }
        UseTree::Glob(_) => {}
    }
}

fn register_std_alias(
    path: &[String],
    requested_alias: String,
    known_aliases: &BTreeSet<String>,
    discovered: &mut BTreeSet<String>,
) {
    let imported_path = path.strip_suffix(&["self".to_owned()]).unwrap_or(path);
    let Some(imported_name) = imported_path.last() else {
        return;
    };
    if !known_aliases.contains(imported_name) {
        return;
    }
    let alias = if requested_alias == "self" {
        imported_name.clone()
    } else {
        requested_alias
    };
    discovered.insert(alias);
}

fn use_tree_reaches_std_fs(
    tree: &UseTree,
    prefix: &mut Vec<String>,
    aliases: &BTreeSet<String>,
) -> bool {
    match tree {
        UseTree::Path(path) => {
            prefix.push(identifier_name(&path.ident));
            let reaches = use_tree_reaches_std_fs(&path.tree, prefix, aliases);
            prefix.pop();
            reaches
        }
        UseTree::Name(name) => {
            prefix.push(identifier_name(&name.ident));
            let reaches = segments_reach_std_fs(prefix, aliases);
            prefix.pop();
            reaches
        }
        UseTree::Rename(rename) => {
            prefix.push(identifier_name(&rename.ident));
            let reaches = segments_reach_std_fs(prefix, aliases);
            prefix.pop();
            reaches
        }
        UseTree::Group(group) => group
            .items
            .iter()
            .any(|tree| use_tree_reaches_std_fs(tree, prefix, aliases)),
        UseTree::Glob(_) => {
            segments_reach_std_fs(prefix, aliases)
                || prefix.last().is_some_and(|name| aliases.contains(name))
        }
    }
}

fn segments_reach_std_fs(segments: &[String], aliases: &BTreeSet<String>) -> bool {
    segments
        .windows(2)
        .any(|window| aliases.contains(&window[0]) && window[1] == "fs")
}

struct ProductionFilesystemVisitor<'a> {
    aliases: BTreeSet<String>,
    safe_macros: BTreeSet<String>,
    source_label: &'a str,
    package: &'a str,
    violations: &'a mut Vec<String>,
}

impl ProductionFilesystemVisitor<'_> {
    fn report(&mut self, path: &str) {
        self.violations.push(format!(
            "native-fs source policy: direct host filesystem path {path} in {} ({}) bypasses baozi-io",
            self.source_label, self.package
        ));
    }

    fn report_unverified_include(&mut self) {
        self.violations.push(format!(
            "native-fs source policy: include! in {} ({}) injects Rust source outside the verified production module graph",
            self.source_label, self.package
        ));
    }

    fn report_unverified_local_module(&mut self, module: &ItemMod) {
        self.violations.push(format!(
            "native-fs source policy: block-local module {} in {} ({}) has no verified source traversal",
            module.ident, self.source_label, self.package
        ));
    }

    fn report_unverified_item_macro(&mut self, item: &syn::Macro) {
        let name = item.path.segments.last().map_or_else(
            || "<unknown>".to_owned(),
            |segment| identifier_name(&segment.ident),
        );
        self.violations.push(format!(
            "native-fs source policy: item macro {}! in {} ({}) can inject unverified production source",
            name,
            self.source_label,
            self.package
        ));
    }
}

fn extend_block_std_aliases(block: &syn::Block, aliases: &mut BTreeSet<String>) {
    for statement in &block.stmts {
        let syn::Stmt::Item(item) = statement else {
            continue;
        };
        if item_is_test_only(item) {
            continue;
        }
        if let Item::ExternCrate(item_extern) = item
            && identifier_is(&item_extern.ident, "std")
        {
            aliases.insert(
                item_extern
                    .rename
                    .as_ref()
                    .map_or_else(|| "std".to_owned(), |(_, alias)| identifier_name(alias)),
            );
        }
    }

    loop {
        let known_aliases = aliases.clone();
        let mut discovered = BTreeSet::new();
        for statement in &block.stmts {
            let syn::Stmt::Item(item) = statement else {
                continue;
            };
            if item_is_test_only(item) {
                continue;
            }
            let Item::Use(item_use) = item else {
                continue;
            };
            collect_std_aliases_from_use_tree(
                &item_use.tree,
                &mut Vec::new(),
                &known_aliases,
                &mut discovered,
            );
        }
        let previous_len = aliases.len();
        aliases.extend(discovered);
        if aliases.len() == previous_len {
            break;
        }
    }
}

impl<'ast> Visit<'ast> for ProductionFilesystemVisitor<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        if !item_is_test_only(item) {
            syn::visit::visit_item(self, item);
        }
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let segments = path
            .segments
            .iter()
            .map(|segment| identifier_name(&segment.ident))
            .collect::<Vec<_>>();
        if segments_reach_std_fs(&segments, &self.aliases) {
            self.report(&segments.join("::"));
            return;
        }
        syn::visit::visit_path(self, path);
    }

    fn visit_macro(&mut self, item: &'ast syn::Macro) {
        if path_ends_in_identifier(&item.path, "include") {
            self.report_unverified_include();
        }
        if let Some(hazard) = macro_expression_hazard(&item.tokens) {
            self.report(hazard);
        }
        syn::visit::visit_macro(self, item);
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if path_is_identifier(&item.mac.path, "macro_rules") {
            if !macro_tokens_are_source_closed(&item.mac.tokens) {
                if let Some(hazard) = macro_source_hazard(&item.mac.tokens) {
                    self.report(hazard);
                } else {
                    self.report_unverified_item_macro(&item.mac);
                }
            }
            return;
        }
        let is_verified = macro_tokens_are_source_closed(&item.mac.tokens)
            && simple_macro_name(&item.mac).is_some_and(|name| self.safe_macros.contains(&name));
        if !is_verified {
            self.report_unverified_item_macro(&item.mac);
        }
        self.visit_macro(&item.mac);
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if attributes_are_test_only(&item.attrs) {
            return;
        }
        if let Some(attribute) = module_source_override(&item.attrs) {
            self.violations.push(format!(
                "native-fs source policy: block-local module {} in {} ({}) uses forbidden {attribute} source redirection",
                item.ident, self.source_label, self.package
            ));
            return;
        }
        let Some((_, items)) = &item.content else {
            self.report_unverified_local_module(item);
            return;
        };

        let outer_aliases = self.aliases.clone();
        let outer_macros = self.safe_macros.clone();
        self.aliases = std_aliases_from_items(items, &self.aliases);
        self.safe_macros = safe_declarative_macros_from_items(items, &self.safe_macros);
        for nested in items {
            if !item_is_test_only(nested) {
                self.visit_item(nested);
            }
        }
        self.aliases = outer_aliases;
        self.safe_macros = outer_macros;
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        if use_tree_mentions_identifier(&item.tree, "include") {
            self.report_unverified_include();
        }
        if use_tree_reaches_std_fs(&item.tree, &mut Vec::new(), &self.aliases) {
            self.report("std::fs import");
        }

        let known_aliases = self.aliases.clone();
        let mut discovered = BTreeSet::new();
        collect_std_aliases_from_use_tree(
            &item.tree,
            &mut Vec::new(),
            &known_aliases,
            &mut discovered,
        );
        for alias in discovered.iter().filter(|alias| alias.as_str() != "std") {
            self.report(&format!("renamed std namespace {alias}"));
        }
        self.aliases.extend(discovered);
    }

    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        if identifier_is(&item.ident, "std") {
            let alias = item
                .rename
                .as_ref()
                .map_or_else(|| "std".to_owned(), |(_, alias)| identifier_name(alias));
            if alias != "std" {
                self.report(&format!("renamed std namespace {alias}"));
            }
            self.aliases.insert(alias);
        }
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        let outer_aliases = self.aliases.clone();
        let outer_macros = self.safe_macros.clone();
        extend_block_std_aliases(block, &mut self.aliases);
        for statement in &block.stmts {
            if let syn::Stmt::Item(item) = statement {
                extend_safe_declarative_macro(item, &mut self.safe_macros);
            }
        }
        syn::visit::visit_block(self, block);
        self.aliases = outer_aliases;
        self.safe_macros = outer_macros;
    }
}

fn resolve_module_file(module_directory: &Path, item: &ItemMod) -> Option<PathBuf> {
    let name = identifier_name(&item.ident);
    [
        module_directory.join(format!("{name}.rs")),
        module_directory.join(name).join("mod.rs"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

fn module_source_override(attributes: &[Attribute]) -> Option<&'static str> {
    for attribute in attributes {
        if path_is_identifier(attribute.path(), "path") {
            return Some("#[path]");
        }
        if !path_is_identifier(attribute.path(), "cfg_attr") {
            continue;
        }
        let Meta::List(list) = &attribute.meta else {
            return Some("unverifiable #[cfg_attr]");
        };
        let Some(arguments) = parse_meta_arguments(list) else {
            return Some("unverifiable #[cfg_attr]");
        };
        if arguments.iter().skip(1).any(meta_can_override_module_path) {
            return Some("#[cfg_attr(..., path = ...)]");
        }
    }
    None
}

fn meta_can_override_module_path(meta: &Meta) -> bool {
    if path_is_identifier(meta.path(), "path") {
        return true;
    }
    let Meta::List(list) = meta else {
        return false;
    };
    if !path_is_identifier(&list.path, "cfg_attr") {
        return false;
    }
    parse_meta_arguments(list)
        .is_none_or(|arguments| arguments.iter().skip(1).any(meta_can_override_module_path))
}

fn module_directory_for_file(source: &Path) -> PathBuf {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    if matches!(
        source.file_name().and_then(|name| name.to_str()),
        Some("mod.rs" | "lib.rs" | "main.rs")
    ) {
        parent.to_path_buf()
    } else {
        source
            .file_stem()
            .map_or_else(|| parent.to_path_buf(), |stem| parent.join(stem))
    }
}

fn display_source_path(repository_root: &Path, path: &Path) -> String {
    path.strip_prefix(repository_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn item_is_test_only(item: &Item) -> bool {
    attributes_are_test_only(item_attributes(item))
}

fn attributes_are_test_only(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        path_is_identifier(attribute.path(), "test")
            || path_is_identifier(attribute.path(), "bench")
            || cfg_requires_test(attribute)
    })
}

fn item_attributes(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        Item::Verbatim(_) => &[],
        _ => &[],
    }
}

fn cfg_requires_test(attribute: &Attribute) -> bool {
    if !path_is_identifier(attribute.path(), "cfg") {
        return false;
    }
    let Meta::List(list) = &attribute.meta else {
        return false;
    };
    let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
    let Ok(arguments) = parser.parse2(list.tokens.clone()) else {
        return false;
    };
    !arguments.is_empty() && arguments.iter().all(meta_requires_test)
}

fn meta_requires_test(meta: &Meta) -> bool {
    match meta {
        Meta::Path(path) => path_is_identifier(path, "test"),
        Meta::List(list) if path_is_identifier(&list.path, "all") => parse_meta_arguments(list)
            .is_some_and(|arguments| arguments.iter().any(meta_requires_test)),
        Meta::List(list) if path_is_identifier(&list.path, "any") => parse_meta_arguments(list)
            .is_some_and(|arguments| {
                !arguments.is_empty() && arguments.iter().all(meta_requires_test)
            }),
        Meta::List(_) | Meta::NameValue(_) => false,
    }
}

fn parse_meta_arguments(list: &syn::MetaList) -> Option<Punctuated<Meta, Token![,]>> {
    Punctuated::<Meta, Token![,]>::parse_terminated
        .parse2(list.tokens.clone())
        .ok()
}

/// Checks already-loaded Cargo metadata documents. This is public for adversarial fixture tests.
#[doc(hidden)]
pub fn check_metadata_fixture(
    manifest_metadata: &Value,
    lockfile: &str,
    baseline: &Value,
    target_metadata: &BTreeMap<String, Value>,
) -> NativeFsReport {
    let mut violations = Vec::new();
    check_manifest(manifest_metadata, &mut violations);
    let checksums = match parse_lockfile_checksums(lockfile) {
        Ok(checksums) => checksums,
        Err(violation) => {
            violations.push(violation);
            return NativeFsReport {
                profiles_checked: target_metadata.len(),
                violations,
            };
        }
    };

    let snapshot = build_snapshot(target_metadata, &checksums, &mut violations);
    if let Some(snapshot) = &snapshot {
        check_baseline_shape(baseline, &mut violations);
        let difference_start = violations.len();
        collect_differences(
            baseline,
            snapshot,
            "baseline",
            difference_start,
            &mut violations,
        );
    }

    violations.sort_unstable();
    violations.dedup();
    NativeFsReport {
        profiles_checked: target_metadata.len(),
        violations,
    }
}

/// Checks independent runtime and repository native-filesystem closure fixtures.
#[doc(hidden)]
pub fn check_metadata_fixture_with_repository(
    manifest_metadata: &Value,
    lockfile: &str,
    baseline: &Value,
    repository_baseline: &Value,
    target_metadata: &BTreeMap<String, Value>,
    profiles: RepositoryMetadataProfiles<'_>,
) -> NativeFsReport {
    let mut report = check_metadata_fixture(manifest_metadata, lockfile, baseline, target_metadata);
    check_complete_local_repository_declarations(
        profiles.local_declarations,
        &mut report.violations,
    );
    check_facade_candidate_profiles(profiles.facade, &mut report.violations);
    if let Err(violation) =
        check_xtask_empty_profile(XTASK_REPOSITORY_RESOLUTION_TARGET, profiles.xtask_empty)
    {
        report.violations.push(violation);
    }
    if let Err(violation) =
        check_xtask_repository_profile(XTASK_REPOSITORY_RESOLUTION_TARGET, profiles.xtask_all)
    {
        report.violations.push(violation);
    }
    let checksums = match parse_lockfile_checksums(lockfile) {
        Ok(checksums) => checksums,
        Err(violation) => {
            report.violations.push(violation);
            report.profiles_checked += profiles.repository.len() + profiles.facade.len() + 2;
            report.violations.sort_unstable();
            report.violations.dedup();
            return report;
        }
    };

    if let Some(snapshot) = build_repository_delta_snapshot(
        baseline,
        profiles.repository,
        &checksums,
        &mut report.violations,
    ) {
        check_repository_baseline_shape(repository_baseline, &mut report.violations);
        let difference_start = report.violations.len();
        collect_differences(
            repository_baseline,
            &snapshot,
            "repository baseline",
            difference_start,
            &mut report.violations,
        );
    }

    report.profiles_checked += profiles.repository.len() + profiles.facade.len() + 2;
    report.violations.sort_unstable();
    report.violations.dedup();
    report
}

/// Builds the canonical resolved-closure document for fixture and maintenance tooling.
#[doc(hidden)]
pub fn resolved_closure_snapshot(
    target_metadata: &BTreeMap<String, Value>,
    lockfile: &str,
) -> Result<Value, Vec<String>> {
    let mut violations = Vec::new();
    let checksums = match parse_lockfile_checksums(lockfile) {
        Ok(checksums) => checksums,
        Err(violation) => {
            return Err(vec![violation]);
        }
    };
    let snapshot = build_snapshot(target_metadata, &checksums, &mut violations);
    violations.sort_unstable();
    violations.dedup();
    match (snapshot, violations.is_empty()) {
        (Some(snapshot), true) => Ok(snapshot),
        _ => Err(violations),
    }
}

/// Builds the repository-only closure delta relative to the frozen runtime baseline.
#[doc(hidden)]
pub fn repository_resolved_closure_snapshot(
    runtime_baseline: &Value,
    repository_target_metadata: &BTreeMap<String, Value>,
    lockfile: &str,
) -> Result<Value, Vec<String>> {
    let mut violations = Vec::new();
    let checksums = match parse_lockfile_checksums(lockfile) {
        Ok(checksums) => checksums,
        Err(violation) => {
            return Err(vec![violation]);
        }
    };
    let snapshot = build_repository_delta_snapshot(
        runtime_baseline,
        repository_target_metadata,
        &checksums,
        &mut violations,
    );
    violations.sort_unstable();
    violations.dedup();
    match (snapshot, violations.is_empty()) {
        (Some(snapshot), true) => Ok(snapshot),
        _ => Err(violations),
    }
}

fn load_metadata(
    repository_root: &Path,
    manifest_path: &str,
    target: Option<&str>,
    features: Option<&str>,
    no_deps: bool,
) -> Result<Value, String> {
    load_metadata_inner(
        repository_root,
        manifest_path,
        target,
        features,
        false,
        no_deps,
    )
}

fn load_metadata_all_features(
    repository_root: &Path,
    manifest_path: &str,
    target: Option<&str>,
    no_deps: bool,
) -> Result<Value, String> {
    load_metadata_inner(repository_root, manifest_path, target, None, true, no_deps)
}

fn load_metadata_inner(
    repository_root: &Path,
    manifest_path: &str,
    target: Option<&str>,
    features: Option<&str>,
    all_features: bool,
    no_deps: bool,
) -> Result<Value, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let manifest = repository_root.join(manifest_path);
    let mut command = Command::new(cargo);
    command
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--manifest-path",
        ])
        .arg(manifest)
        .current_dir(repository_root);
    if all_features {
        command.arg("--all-features");
    } else {
        command.arg("--no-default-features");
    }
    if no_deps {
        command.arg("--no-deps");
    }
    if let Some(target) = target {
        command.args(["--filter-platform", target]);
    }
    if let Some(features) = features {
        command.args(["--features", features]);
    }

    let output = command.output().map_err(|error| {
        let target = target.unwrap_or("manifest");
        format!("native-fs metadata {target}: failed to run cargo metadata: {error}")
    })?;
    if !output.status.success() {
        let target = target.unwrap_or("manifest");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "native-fs metadata {target}: cargo metadata --locked failed: {}",
            diagnostic.trim()
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        let target = target.unwrap_or("manifest");
        format!("native-fs metadata {target}: cargo returned invalid JSON: {error}")
    })
}

fn read_json(path: &Path, label: &str) -> Result<Value, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("{label}: failed to read {path:?}: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{label}: invalid JSON: {error}"))
}

fn read_lockfile(repository_root: &Path) -> Result<String, String> {
    let path = repository_root.join("Cargo.lock");
    fs::read_to_string(&path)
        .map_err(|error| format!("native-fs lockfile: failed to read {path:?}: {error}"))
}

type ChecksumKey = (String, String, String);

fn parse_lockfile_checksums(lockfile: &str) -> Result<BTreeMap<ChecksumKey, String>, String> {
    let document: toml::Value = toml::from_str(lockfile)
        .map_err(|error| format!("native-fs lockfile: invalid TOML: {error}"))?;
    let packages = document
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "native-fs lockfile: omitted package array".to_owned())?;
    let mut checksums = BTreeMap::new();
    for package in packages {
        let Some(source) = package.get("source").and_then(toml::Value::as_str) else {
            continue;
        };
        let name = package
            .get("name")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| "native-fs lockfile: package omitted name".to_owned())?;
        let version = package
            .get("version")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("native-fs lockfile: package {name} omitted version"))?;
        let checksum = package
            .get("checksum")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| {
                format!("native-fs lockfile: registry package {name}@{version} omitted checksum")
            })?;
        let key = (name.to_owned(), version.to_owned(), source.to_owned());
        if checksums.insert(key, checksum.to_owned()).is_some() {
            return Err(format!(
                "native-fs lockfile: duplicate registry package {name}@{version} from {source}"
            ));
        }
    }
    Ok(checksums)
}

fn check_complete_local_repository_declarations(metadata: &Value, violations: &mut Vec<String>) {
    const LABEL: &str = "native-fs complete local declarations";
    const XTASK_MANIFEST: &str = "crates/baozi-xtask/Cargo.toml";

    let Some(packages) = metadata.get("packages").and_then(Value::as_array) else {
        violations.push(format!("{LABEL}: metadata omitted packages"));
        return;
    };
    let Some(workspace_root) = metadata.get("workspace_root").and_then(Value::as_str) else {
        violations.push(format!("{LABEL}: metadata omitted workspace_root"));
        return;
    };
    let expected_xtask_manifest = Path::new(workspace_root).join(XTASK_MANIFEST);
    let mut authentic_xtask_count = 0;

    for package in packages {
        let Some(dependencies) = package.get("dependencies").and_then(Value::as_array) else {
            if package.get("source") == Some(&Value::Null) {
                let owner = string_field(package, "name").unwrap_or("<unnamed>");
                violations.push(format!(
                    "{LABEL}: local package {owner} omitted dependencies"
                ));
            }
            continue;
        };
        if package.get("source") != Some(&Value::Null) {
            let owner = string_field(package, "name").unwrap_or("<unnamed>");
            check_unregistered_repository_feature_requests(
                owner,
                package,
                dependencies,
                violations,
            );
            continue;
        }
        let Some(owner) = string_field(package, "name") else {
            violations.push(format!("{LABEL}: local package omitted its name"));
            continue;
        };
        let Some(package_id) = string_field(package, "id") else {
            violations.push(format!("{LABEL}: local package {owner} omitted its id"));
            continue;
        };
        let manifest_matches_xtask = package
            .get("manifest_path")
            .and_then(Value::as_str)
            .is_some_and(|manifest| Path::new(manifest) == expected_xtask_manifest);
        let claims_xtask_identity = owner == XTASK_PACKAGE || manifest_matches_xtask;

        let authentic_xtask = if claims_xtask_identity {
            match check_workspace_package(
                metadata,
                package,
                package_id,
                XTASK_PACKAGE,
                XTASK_MANIFEST,
                LABEL,
            ) {
                Ok(()) => {
                    authentic_xtask_count += 1;
                    true
                }
                Err(violation) => {
                    violations.push(violation);
                    false
                }
            }
        } else {
            false
        };

        if authentic_xtask {
            check_xtask_repository_dependency(package, violations);
        } else {
            check_unregistered_repository_feature_requests(
                owner,
                package,
                dependencies,
                violations,
            );
        }
    }

    if authentic_xtask_count != 1 {
        violations.push(format!(
            "{LABEL}: expected exactly one authentic {XTASK_PACKAGE} declaration, found {authentic_xtask_count}"
        ));
    }
}

fn check_manifest(metadata: &Value, violations: &mut Vec<String>) {
    let Some(workspace_packages) = metadata.get("packages").and_then(Value::as_array) else {
        violations.push("native-fs manifest: metadata omitted packages".to_owned());
        return;
    };
    for workspace_package in workspace_packages {
        let Some(owner) = string_field(workspace_package, "name") else {
            violations.push("native-fs manifest: workspace package omitted its name".to_owned());
            continue;
        };
        if owner == "baozi-io" {
            continue;
        }
        let Some(dependencies) = workspace_package
            .get("dependencies")
            .and_then(Value::as_array)
        else {
            violations.push(format!("native-fs manifest: {owner} omitted dependencies"));
            continue;
        };
        let registered_xtask = if owner == XTASK_PACKAGE {
            match string_field(workspace_package, "id") {
                Some(package_id) => match check_workspace_package(
                    metadata,
                    workspace_package,
                    package_id,
                    XTASK_PACKAGE,
                    "crates/baozi-xtask/Cargo.toml",
                    "native-fs manifest",
                ) {
                    Ok(()) => true,
                    Err(violation) => {
                        violations.push(violation);
                        false
                    }
                },
                None => {
                    violations.push(format!(
                        "native-fs manifest: {XTASK_PACKAGE} omitted its package id"
                    ));
                    false
                }
            }
        } else {
            false
        };
        if !registered_xtask {
            check_unregistered_repository_feature_requests(
                owner,
                workspace_package,
                dependencies,
                violations,
            );
        }
        for dependency in dependencies {
            let Some(name) = string_field(dependency, "name") else {
                violations.push(format!(
                    "native-fs manifest: {owner} has a dependency without a name"
                ));
                continue;
            };
            if NATIVE_FILESYSTEM_PACKAGES.contains(&name) {
                violations.push(format!(
                    "native-fs manifest: {owner} must not depend directly on {name}; native filesystem dependencies are private to baozi-io"
                ));
            }
        }
    }

    let Some(xtask) = find_unique_package(metadata, XTASK_PACKAGE, "manifest", violations) else {
        return;
    };
    check_xtask_repository_dependency(xtask, violations);

    let Some(package) = find_unique_package(metadata, "baozi-io", "manifest", violations) else {
        return;
    };

    let Some(features) = package.get("features").and_then(Value::as_object) else {
        violations.push("native-fs manifest: baozi-io omitted its feature table".to_owned());
        return;
    };
    let actual_default_features = features
        .get("default")
        .and_then(Value::as_array)
        .and_then(|values| sorted_strings(values).ok());
    if actual_default_features.as_deref() != Some(&[]) {
        violations.push(format!(
            "native-fs manifest: baozi-io/default must be empty, found {actual_default_features:?}"
        ));
    }
    if features.contains_key("fs") {
        violations.push(
            "native-fs manifest: baozi-io/fs must remain unregistered until conformance promotion"
                .to_owned(),
        );
    }
    let actual_fs_features = features
        .get("fs-candidate")
        .and_then(Value::as_array)
        .and_then(|values| sorted_strings(values).ok());
    let expected_fs_features = vec!["dep:cap-std".to_owned(), "dep:rustix".to_owned()];
    if actual_fs_features.as_ref() != Some(&expected_fs_features) {
        violations.push(format!(
            "native-fs manifest: baozi-io/fs-candidate must be exactly {expected_fs_features:?}, found {actual_fs_features:?}"
        ));
    }
    let actual_repository_features = features
        .get("repository-fs-candidate")
        .and_then(Value::as_array)
        .and_then(|values| sorted_strings(values).ok());
    let expected_repository_features = vec!["dep:cap-fs-ext".to_owned(), "fs-candidate".to_owned()];
    if actual_repository_features.as_ref() != Some(&expected_repository_features) {
        violations.push(format!(
            "native-fs manifest: baozi-io/repository-fs-candidate must be exactly {expected_repository_features:?}, found {actual_repository_features:?}"
        ));
    }

    let Some(dependencies) = package.get("dependencies").and_then(Value::as_array) else {
        violations.push("native-fs manifest: baozi-io omitted dependencies".to_owned());
        return;
    };
    check_baozi_io_production_dependency_set(dependencies, violations);

    if dependencies
        .iter()
        .any(|dependency| string_field(dependency, "name") == Some("cap-primitives"))
    {
        violations.push(
            "native-fs manifest: baozi-io must not depend directly on cap-primitives".to_owned(),
        );
    }

    check_manifest_dependency(
        dependencies,
        ManifestDependency {
            name: "cap-fs-ext",
            requirement: "=4.0.2",
            target: CAP_STD_TARGET,
            features: &["std"],
        },
        violations,
    );
    check_manifest_dependency(
        dependencies,
        ManifestDependency {
            name: "cap-std",
            requirement: "=4.0.2",
            target: CAP_STD_TARGET,
            features: &[],
        },
        violations,
    );

    let Some(facade) = find_unique_package(metadata, "baozi", "manifest", violations) else {
        return;
    };
    let Some(facade_features) = facade.get("features").and_then(Value::as_object) else {
        violations.push("native-fs manifest: baozi omitted its feature table".to_owned());
        return;
    };
    if facade_features.contains_key("native-fs") {
        violations.push(
            "native-fs manifest: baozi/native-fs must remain unregistered until conformance promotion"
                .to_owned(),
        );
    }
    let facade_native_fs = facade_features
        .get("native-fs-candidate")
        .and_then(Value::as_array)
        .and_then(|values| sorted_strings(values).ok());
    let expected_facade_native_fs = vec!["baozi-io/fs-candidate".to_owned(), "std".to_owned()];
    if facade_native_fs.as_ref() != Some(&expected_facade_native_fs) {
        violations.push(format!(
            "native-fs manifest: baozi/native-fs-candidate must be exactly {expected_facade_native_fs:?}, found {facade_native_fs:?}"
        ));
    }
    match feature_reaches_repository_fs(facade_features, "native-fs-candidate") {
        Ok(false) => {}
        Ok(true) => violations.push(
            "native-fs manifest: baozi/native-fs-candidate must not reach baozi-io/repository-fs-candidate"
                .to_owned(),
        ),
        Err(violation) => violations.push(violation),
    }
    for portable_feature in ["default", "all-formats"] {
        match feature_reaches_native_fs(facade_features, portable_feature) {
            Ok(false) => {}
            Ok(true) => violations.push(format!(
                "native-fs manifest: baozi/{portable_feature} must not enable the native filesystem adapter"
            )),
            Err(violation) => violations.push(violation),
        }
    }
    check_manifest_dependency(
        dependencies,
        ManifestDependency {
            name: "rustix",
            requirement: "=1.1.4",
            target: RUSTIX_TARGET,
            features: &["fs", "std"],
        },
        violations,
    );
}

fn check_baozi_io_production_dependency_set(dependencies: &[Value], violations: &mut Vec<String>) {
    let expected = BTreeSet::from([
        "cap-fs-ext".to_owned(),
        "cap-std".to_owned(),
        "rustix".to_owned(),
        "thiserror".to_owned(),
    ]);
    let mut actual = BTreeSet::new();
    let mut duplicate = BTreeSet::new();
    for dependency in dependencies {
        match dependency.get("kind") {
            None | Some(Value::Null) => {}
            Some(Value::String(kind)) if kind == "build" => {}
            Some(Value::String(kind)) if kind == "dev" => continue,
            _ => {
                violations.push(
                    "native-fs manifest: baozi-io has a dependency with an unsupported kind"
                        .to_owned(),
                );
                continue;
            }
        }
        let Some(name) = string_field(dependency, "name") else {
            violations.push(
                "native-fs manifest: baozi-io has a production dependency without a name"
                    .to_owned(),
            );
            continue;
        };
        if !actual.insert(name.to_owned()) {
            duplicate.insert(name.to_owned());
        }
    }
    if actual != expected || !duplicate.is_empty() {
        violations.push(format!(
            "native-fs manifest: baozi-io production dependency set must be exactly {expected:?} with no duplicates, found {actual:?}, duplicates {duplicate:?}"
        ));
    }
}

fn check_unregistered_repository_feature_requests(
    owner: &str,
    package: &Value,
    dependencies: &[Value],
    violations: &mut Vec<String>,
) {
    let mut dependency_keys = BTreeSet::new();
    for dependency in dependencies {
        if string_field(dependency, "name") != Some("baozi-io") {
            continue;
        }
        let key = string_field(dependency, "rename").unwrap_or("baozi-io");
        dependency_keys.insert(key.to_owned());
        let requested = dependency
            .get("features")
            .and_then(Value::as_array)
            .and_then(|values| sorted_strings(values).ok());
        if requested.as_ref().is_some_and(|features| {
            features
                .iter()
                .any(|feature| feature == "repository-fs-candidate")
        }) {
            violations.push(format!(
                "native-fs manifest: {owner} must not request baozi-io/repository-fs-candidate on a dependency edge"
            ));
        }
    }
    if dependency_keys.is_empty() {
        return;
    }

    let Some(features) = package.get("features").and_then(Value::as_object) else {
        violations.push(format!(
            "native-fs manifest: {owner} omitted its feature table while depending on baozi-io"
        ));
        return;
    };
    for (feature, values) in features {
        let Some(values) = values.as_array() else {
            violations.push(format!(
                "native-fs manifest: {owner}/{feature} omitted a string feature list"
            ));
            continue;
        };
        for value in values {
            let Some(value) = value.as_str() else {
                violations.push(format!(
                    "native-fs manifest: {owner}/{feature} contains a non-string feature"
                ));
                continue;
            };
            let forwarded_dependency = value
                .strip_suffix("/repository-fs-candidate")
                .map(|key| key.strip_suffix('?').unwrap_or(key));
            if forwarded_dependency.is_some_and(|key| dependency_keys.contains(key)) {
                violations.push(format!(
                    "native-fs manifest: {owner}/{feature} must not forward baozi-io/repository-fs-candidate"
                ));
            }
        }
    }
}

fn check_xtask_repository_dependency(package: &Value, violations: &mut Vec<String>) {
    let Some(feature_table) = package.get("features").and_then(Value::as_object) else {
        violations.push(format!(
            "native-fs manifest: {XTASK_PACKAGE} omitted its feature table"
        ));
        return;
    };
    let default_features = feature_table
        .get("default")
        .and_then(Value::as_array)
        .and_then(|values| sorted_strings(values).ok());
    if default_features.as_deref() != Some(&[]) {
        violations.push(format!(
            "native-fs manifest: {XTASK_PACKAGE}/default must be empty, found {default_features:?}"
        ));
    }
    let repository_feature = feature_table
        .get("repository-input-candidate")
        .and_then(Value::as_array)
        .and_then(|values| sorted_strings(values).ok());
    let expected_repository_feature = vec!["dep:baozi-io".to_owned()];
    if repository_feature.as_ref() != Some(&expected_repository_feature) {
        violations.push(format!(
            "native-fs manifest: {XTASK_PACKAGE}/repository-input-candidate must be exactly {expected_repository_feature:?}, found {repository_feature:?}"
        ));
    }
    for feature in feature_table.keys() {
        if feature == "repository-input-candidate" {
            continue;
        }
        match feature_reaches_dependency(feature_table, feature, "baozi-io", XTASK_PACKAGE) {
            Ok(false) => {}
            Ok(true) => violations.push(format!(
                "native-fs manifest: {XTASK_PACKAGE}/{feature} must not provide an alternate activation path to baozi-io"
            )),
            Err(violation) => violations.push(violation),
        }
    }

    let Some(dependencies) = package.get("dependencies").and_then(Value::as_array) else {
        violations.push(format!(
            "native-fs manifest: {XTASK_PACKAGE} omitted dependencies"
        ));
        return;
    };
    let matching: Vec<_> = dependencies
        .iter()
        .filter(|dependency| string_field(dependency, "name") == Some("baozi-io"))
        .collect();
    let dependency = match matching.as_slice() {
        [dependency] => *dependency,
        _ => {
            violations.push(format!(
                "native-fs manifest: expected exactly one {XTASK_PACKAGE} direct baozi-io repository edge, found {}",
                matching.len()
            ));
            return;
        }
    };
    let features = dependency
        .get("features")
        .and_then(Value::as_array)
        .and_then(|values| sorted_strings(values).ok());
    let expected_features = vec!["repository-fs-candidate".to_owned()];
    let exact = dependency.get("source") == Some(&Value::Null)
        && dependency.get("kind") == Some(&Value::Null)
        && dependency.get("rename") == Some(&Value::Null)
        && dependency.get("optional").and_then(Value::as_bool) == Some(true)
        && dependency
            .get("uses_default_features")
            .and_then(Value::as_bool)
            == Some(false)
        && string_field(dependency, "target") == Some(XTASK_REPOSITORY_TARGET)
        && features.as_ref() == Some(&expected_features);
    if !exact {
        violations.push(format!(
            "native-fs manifest: {XTASK_PACKAGE} direct baozi-io repository edge drifted from path source, normal optional kind, default-features=false, features={expected_features:?}, target={XTASK_REPOSITORY_TARGET}"
        ));
    }
}

fn feature_reaches_native_fs(features: &Map<String, Value>, root: &str) -> Result<bool, String> {
    feature_reaches_any(
        features,
        root,
        &[
            "native-fs",
            "native-fs-candidate",
            "baozi-io/fs",
            "baozi-io?/fs",
            "baozi-io/fs-candidate",
            "baozi-io?/fs-candidate",
            "baozi-io/repository-fs-candidate",
            "baozi-io?/repository-fs-candidate",
        ],
    )
}

fn feature_reaches_repository_fs(
    features: &Map<String, Value>,
    root: &str,
) -> Result<bool, String> {
    feature_reaches_any(
        features,
        root,
        &[
            "baozi-io/repository-fs-candidate",
            "baozi-io?/repository-fs-candidate",
        ],
    )
}

fn feature_reaches_any(
    features: &Map<String, Value>,
    root: &str,
    forbidden: &[&str],
) -> Result<bool, String> {
    let mut pending = VecDeque::from([root.to_owned()]);
    let mut visited = BTreeSet::new();
    while let Some(feature) = pending.pop_front() {
        if !visited.insert(feature.clone()) {
            continue;
        }
        let values = features
            .get(&feature)
            .and_then(Value::as_array)
            .ok_or_else(|| {
                format!("native-fs manifest: baozi/{feature} omitted a string feature list")
            })?;
        for value in values {
            let value = value.as_str().ok_or_else(|| {
                format!("native-fs manifest: baozi/{feature} contains a non-string feature")
            })?;
            if forbidden.contains(&value) {
                return Ok(true);
            }
            if !value.starts_with("dep:") && !value.contains('/') {
                pending.push_back(value.to_owned());
            }
        }
    }
    Ok(false)
}

fn feature_reaches_dependency(
    features: &Map<String, Value>,
    root: &str,
    dependency: &str,
    owner: &str,
) -> Result<bool, String> {
    let mut pending = VecDeque::from([root.to_owned()]);
    let mut visited = BTreeSet::new();
    let explicit_dependency = format!("dep:{dependency}");
    let dependency_feature_prefix = format!("{dependency}/");
    let weak_dependency_feature_prefix = format!("{dependency}?/");
    while let Some(feature) = pending.pop_front() {
        if !visited.insert(feature.clone()) {
            continue;
        }
        let values = features
            .get(&feature)
            .and_then(Value::as_array)
            .ok_or_else(|| {
                format!("native-fs manifest: {owner}/{feature} omitted a string feature list")
            })?;
        for value in values {
            let value = value.as_str().ok_or_else(|| {
                format!("native-fs manifest: {owner}/{feature} contains a non-string feature")
            })?;
            if value == explicit_dependency.as_str()
                || value == dependency
                || value.starts_with(&dependency_feature_prefix)
                || value.starts_with(&weak_dependency_feature_prefix)
            {
                return Ok(true);
            }
            if !value.starts_with("dep:") && !value.contains('/') {
                pending.push_back(value.to_owned());
            }
        }
    }
    Ok(false)
}

fn check_facade_candidate_profiles(
    profiles: &BTreeMap<String, Value>,
    violations: &mut Vec<String>,
) {
    let expected = string_set(REPOSITORY_CANDIDATE_TARGETS);
    let actual = profiles.keys().cloned().collect::<BTreeSet<_>>();
    if actual != expected {
        violations.push(format!(
            "native-fs facade candidate profiles: target set must be {expected:?}, found {actual:?}"
        ));
    }
    for target in REPOSITORY_CANDIDATE_TARGETS {
        let Some(metadata) = profiles.get(*target) else {
            continue;
        };
        if let Err(violation) = check_facade_candidate_profile(target, metadata) {
            violations.push(violation);
        }
    }
}

fn check_facade_candidate_profile(target: &str, metadata: &Value) -> Result<(), String> {
    let label = format!("native-fs facade candidate profile {target}");
    let document = MetadataDocument::parse(metadata, target)?;
    let root = document.unique_package_id(FACADE_PACKAGE)?;
    check_workspace_package(
        metadata,
        document.package(root)?,
        root,
        FACADE_PACKAGE,
        "crates/baozi/Cargo.toml",
        &label,
    )?;
    let root_node = document.node(root)?;
    let root_features = root_node
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{label}: {FACADE_PACKAGE} omitted resolved features"))?;
    let root_features = sorted_strings(root_features)?;
    if !root_features
        .iter()
        .any(|feature| feature == "native-fs-candidate")
    {
        return Err(format!(
            "{label}: {FACADE_PACKAGE} must resolve native-fs-candidate"
        ));
    }

    let reachable = document.production_closure(&[root.to_owned()])?;
    let mut io_packages = Vec::new();
    let mut cap_fs_ext = 0;
    for id in &reachable {
        let package = document.package(id)?;
        match string_field(package, "name") {
            Some("baozi-io") => io_packages.push(id.as_str()),
            Some("cap-fs-ext") => cap_fs_ext += 1,
            _ => {}
        }
    }
    let io_id = match io_packages.as_slice() {
        [id] => *id,
        _ => {
            return Err(format!(
                "{label}: expected exactly one reachable baozi-io package, found {}",
                io_packages.len()
            ));
        }
    };
    check_workspace_package(
        metadata,
        document.package(io_id)?,
        io_id,
        "baozi-io",
        "crates/baozi-io/Cargo.toml",
        &label,
    )?;
    let io_features = document
        .node(io_id)?
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{label}: baozi-io omitted resolved features"))?;
    let io_features = sorted_nondefault_features(io_features)?;
    let expected_io_features = vec!["fs-candidate".to_owned()];
    if io_features != expected_io_features {
        return Err(format!(
            "{label}: baozi-io resolved features must be exactly {expected_io_features:?}, found {io_features:?}"
        ));
    }

    if cap_fs_ext != 0 {
        return Err(format!(
            "{label}: cap-fs-ext is forbidden in the resolved facade candidate closure"
        ));
    }
    Ok(())
}

fn check_xtask_empty_profile(target: &str, metadata: &Value) -> Result<(), String> {
    let label = format!("native-fs xtask empty profile {target}");
    let document = MetadataDocument::parse(metadata, target)?;
    let xtask_id = document.unique_package_id(XTASK_PACKAGE)?;
    check_workspace_package(
        metadata,
        document.package(xtask_id)?,
        xtask_id,
        XTASK_PACKAGE,
        "crates/baozi-xtask/Cargo.toml",
        &label,
    )?;
    let xtask_node = document.node(xtask_id)?;
    let features = xtask_node
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{label}: {XTASK_PACKAGE} omitted resolved features"))?;
    if !sorted_nondefault_features(features)?.is_empty() {
        return Err(format!("{label}: {XTASK_PACKAGE} must resolve no features"));
    }
    let reachable = document.production_closure(&[xtask_id.to_owned()])?;
    for id in reachable {
        if string_field(document.package(&id)?, "name") == Some("baozi-io") {
            return Err(format!(
                "{label}: empty-feature xtask must not reach baozi-io"
            ));
        }
    }
    Ok(())
}

fn check_xtask_repository_profile(target: &str, metadata: &Value) -> Result<(), String> {
    let label = format!("native-fs xtask repository profile {target}");
    let document = MetadataDocument::parse(metadata, target)?;
    let xtask_id = document.unique_package_id(XTASK_PACKAGE)?;
    let io_id = document.unique_package_id("baozi-io")?;
    check_workspace_package(
        metadata,
        document.package(xtask_id)?,
        xtask_id,
        XTASK_PACKAGE,
        "crates/baozi-xtask/Cargo.toml",
        &label,
    )?;
    check_workspace_package(
        metadata,
        document.package(io_id)?,
        io_id,
        "baozi-io",
        "crates/baozi-io/Cargo.toml",
        &label,
    )?;

    let xtask_node = document.node(xtask_id)?;
    let xtask_features = xtask_node
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{label}: {XTASK_PACKAGE} omitted resolved features"))?;
    let xtask_features = sorted_nondefault_features(xtask_features)?;
    let expected_xtask_features = vec!["repository-input-candidate".to_owned()];
    if xtask_features != expected_xtask_features {
        return Err(format!(
            "{label}: {XTASK_PACKAGE} resolved features must be exactly {expected_xtask_features:?}, found {xtask_features:?}"
        ));
    }
    let direct_io_edges = production_dependencies(xtask_node, target)?
        .into_iter()
        .filter(|dependency| dependency.package == io_id)
        .collect::<Vec<_>>();
    if direct_io_edges.len() != 1 {
        return Err(format!(
            "{label}: {XTASK_PACKAGE} must have exactly one resolved direct edge to workspace baozi-io, found {}",
            direct_io_edges.len()
        ));
    }
    let reachable = document.production_closure(&[xtask_id.to_owned()])?;
    let mut incoming_io_edges = Vec::new();
    for parent_id in &reachable {
        for dependency in production_dependencies(document.node(parent_id)?, target)? {
            if dependency.package == io_id {
                incoming_io_edges.push((parent_id.as_str(), dependency.name));
            }
        }
        let package = document.package(parent_id)?;
        if package.get("source") == Some(&Value::Null)
            && parent_id != xtask_id
            && parent_id != io_id
        {
            return Err(format!(
                "{label}: unexpected local package {} is reachable from xtask",
                required_string(package, "name", "package")?
            ));
        }
    }
    if incoming_io_edges.len() != 1 || incoming_io_edges[0].0 != xtask_id {
        return Err(format!(
            "{label}: workspace baozi-io must have exactly one reachable incoming edge owned by xtask, found {incoming_io_edges:?}"
        ));
    }
    let io_package = document.package(io_id)?;
    if io_package.get("source") != Some(&Value::Null) {
        return Err(format!(
            "{label}: resolved baozi-io must be a workspace path package"
        ));
    }
    let io_features = document
        .node(io_id)?
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{label}: baozi-io omitted resolved features"))?;
    let io_features = sorted_nondefault_features(io_features)?;
    let expected_io_features = vec![
        "fs-candidate".to_owned(),
        "repository-fs-candidate".to_owned(),
    ];
    if io_features != expected_io_features {
        return Err(format!(
            "{label}: workspace baozi-io resolved features must be exactly {expected_io_features:?}, found {io_features:?}"
        ));
    }
    Ok(())
}

fn check_workspace_package(
    metadata: &Value,
    package: &Value,
    package_id: &str,
    expected_name: &str,
    relative_manifest: &str,
    label: &str,
) -> Result<(), String> {
    if package.get("source") != Some(&Value::Null) {
        return Err(format!(
            "{label}: package {package_id} must be a workspace path package"
        ));
    }
    if string_field(package, "id") != Some(package_id) {
        return Err(format!(
            "{label}: package identity must contain id {package_id}"
        ));
    }
    let actual_name = string_field(package, "name")
        .ok_or_else(|| format!("{label}: package {package_id} omitted name"))?;
    if actual_name != expected_name {
        return Err(format!(
            "{label}: package {package_id} name must be {expected_name}, found {actual_name}"
        ));
    }
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{label}: metadata omitted packages"))?;
    let package_count = packages
        .iter()
        .filter(|candidate| string_field(candidate, "id") == Some(package_id))
        .count();
    if package_count != 1 {
        return Err(format!(
            "{label}: package {package_id} must appear exactly once in packages, found {package_count}"
        ));
    }
    let workspace_root = metadata
        .get("workspace_root")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label}: metadata omitted workspace_root"))?;
    let manifest_path = package
        .get("manifest_path")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label}: package {package_id} omitted manifest_path"))?;
    let expected_manifest = Path::new(workspace_root).join(relative_manifest);
    if Path::new(manifest_path) != expected_manifest {
        return Err(format!(
            "{label}: package {package_id} manifest_path must be {}, found {manifest_path}",
            expected_manifest.display()
        ));
    }
    let workspace_members = metadata
        .get("workspace_members")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{label}: metadata omitted workspace_members"))?;
    let member_count = workspace_members
        .iter()
        .filter(|member| member.as_str() == Some(package_id))
        .count();
    if member_count != 1 {
        return Err(format!(
            "{label}: package {package_id} must appear exactly once in workspace_members, found {member_count}"
        ));
    }
    Ok(())
}

struct ManifestDependency<'a> {
    name: &'a str,
    requirement: &'a str,
    target: &'a str,
    features: &'a [&'a str],
}

fn check_manifest_dependency(
    dependencies: &[Value],
    expected: ManifestDependency<'_>,
    violations: &mut Vec<String>,
) {
    let matching: Vec<_> = dependencies
        .iter()
        .filter(|dependency| string_field(dependency, "name") == Some(expected.name))
        .collect();
    let dependency = match matching.as_slice() {
        [dependency] => *dependency,
        _ => {
            violations.push(format!(
                "native-fs manifest: expected exactly one direct {} edge, found {}",
                expected.name,
                matching.len()
            ));
            return;
        }
    };
    let actual_features = dependency
        .get("features")
        .and_then(Value::as_array)
        .and_then(|values| sorted_strings(values).ok());
    let mut expected_features: Vec<_> = expected
        .features
        .iter()
        .map(|feature| (*feature).to_owned())
        .collect();
    expected_features.sort_unstable();

    let exact = string_field(dependency, "source") == Some(CRATES_IO_SOURCE)
        && string_field(dependency, "req") == Some(expected.requirement)
        && dependency.get("kind") == Some(&Value::Null)
        && dependency.get("rename") == Some(&Value::Null)
        && dependency.get("optional").and_then(Value::as_bool) == Some(true)
        && dependency
            .get("uses_default_features")
            .and_then(Value::as_bool)
            == Some(false)
        && string_field(dependency, "target") == Some(expected.target)
        && actual_features.as_ref() == Some(&expected_features);
    if !exact {
        violations.push(format!(
            "native-fs manifest: direct {} edge drifted from source={CRATES_IO_SOURCE}, req={}, optional=true, default-features=false, features={expected_features:?}, target={}",
            expected.name, expected.requirement, expected.target
        ));
    }
}

fn build_snapshot(
    target_metadata: &BTreeMap<String, Value>,
    checksums: &BTreeMap<ChecksumKey, String>,
    violations: &mut Vec<String>,
) -> Option<Value> {
    let mut identities = BTreeMap::new();
    let mut dependency_audit_profiles = Map::new();
    let mut complete = true;

    for profile in DEPENDENCY_AUDIT_PROFILES {
        let Some(metadata) = target_metadata.get(profile.target) else {
            violations.push(format!(
                "native-fs profile {}: missing cargo metadata for {}",
                profile.name, profile.target
            ));
            complete = false;
            continue;
        };
        match build_dependency_audit_profile(
            DependencyAuditKind::Runtime,
            *profile,
            metadata,
            checksums,
            &mut identities,
            violations,
        ) {
            Ok(snapshot) => {
                dependency_audit_profiles.insert(profile.name.to_owned(), snapshot);
            }
            Err(violation) => {
                violations.push(violation);
                complete = false;
            }
        }
    }

    for target in PORTABLE_TARGETS {
        let Some(metadata) = target_metadata.get(*target) else {
            violations.push(format!(
                "native-fs portable profile {target}: missing cargo metadata"
            ));
            complete = false;
            continue;
        };
        if let Err(violation) = check_portable_profile(target, metadata) {
            violations.push(violation);
        }
    }

    if !complete {
        return None;
    }

    let identities = identities.into_iter().collect::<Map<_, _>>();
    Some(Value::Object(Map::from_iter([
        ("schema".to_owned(), Value::from(3)),
        ("review_basis".to_owned(), review_basis()),
        (
            "metadata_semantics".to_owned(),
            Value::String(
                "cargo metadata --locked --filter-platform; normal/build closure only".to_owned(),
            ),
        ),
        ("package_identities".to_owned(), Value::Object(identities)),
        (
            "dependency_audit_profiles".to_owned(),
            Value::Object(dependency_audit_profiles),
        ),
        (
            "release_registered_targets".to_owned(),
            string_array(RELEASE_REGISTERED_TARGETS),
        ),
        (
            "runtime_candidate_targets".to_owned(),
            string_array(RUNTIME_CANDIDATE_TARGETS),
        ),
        (
            "portable_byte_targets".to_owned(),
            Value::Array(
                PORTABLE_TARGETS
                    .iter()
                    .map(|target| Value::String((*target).to_owned()))
                    .collect(),
            ),
        ),
    ])))
}

fn build_repository_delta_snapshot(
    runtime_baseline: &Value,
    target_metadata: &BTreeMap<String, Value>,
    checksums: &BTreeMap<ChecksumKey, String>,
    violations: &mut Vec<String>,
) -> Option<Value> {
    let Some(runtime_identities) = runtime_baseline
        .get("package_identities")
        .and_then(Value::as_object)
    else {
        violations.push(
            "native-fs repository baseline: runtime baseline omitted package identities".to_owned(),
        );
        return None;
    };
    let Some(runtime_profiles) = runtime_baseline
        .get("dependency_audit_profiles")
        .and_then(Value::as_object)
    else {
        violations.push(
            "native-fs repository baseline: runtime baseline omitted dependency audit profiles"
                .to_owned(),
        );
        return None;
    };

    let mut repository_identities = BTreeMap::new();
    let mut repository_profiles = Map::new();
    let mut complete = true;
    for profile in DEPENDENCY_AUDIT_PROFILES {
        let Some(metadata) = target_metadata.get(profile.target) else {
            violations.push(format!(
                "native-fs repository profile {}: missing cargo metadata for {}",
                profile.name, profile.target
            ));
            complete = false;
            continue;
        };
        match build_dependency_audit_profile(
            DependencyAuditKind::Repository,
            *profile,
            metadata,
            checksums,
            &mut repository_identities,
            violations,
        ) {
            Ok(snapshot) => {
                repository_profiles.insert(profile.name.to_owned(), snapshot);
            }
            Err(violation) => {
                violations.push(violation);
                complete = false;
            }
        }
    }
    if !complete {
        return None;
    }

    let repository_identities = repository_identities.into_iter().collect::<Map<_, _>>();
    let mut profile_deltas = Map::new();
    for profile in DEPENDENCY_AUDIT_PROFILES {
        let Some(runtime_profile) = runtime_profiles
            .get(profile.name)
            .and_then(Value::as_object)
        else {
            violations.push(format!(
                "native-fs repository baseline: runtime baseline omitted profile {}",
                profile.name
            ));
            complete = false;
            continue;
        };
        let Some(repository_profile) = repository_profiles
            .get(profile.name)
            .and_then(Value::as_object)
        else {
            violations.push(format!(
                "native-fs repository baseline: repository snapshot omitted profile {}",
                profile.name
            ));
            complete = false;
            continue;
        };
        let roots = match value_string_set_delta(
            runtime_profile.get("roots"),
            repository_profile.get("roots"),
            &format!("native-fs repository profile {} roots", profile.name),
        ) {
            Ok(delta) => delta,
            Err(violation) => {
                violations.push(violation);
                complete = false;
                continue;
            }
        };
        let full_package_keys = match value_string_set_delta(
            runtime_profile.get("full_package_keys"),
            repository_profile.get("full_package_keys"),
            &format!(
                "native-fs repository profile {} full package keys",
                profile.name
            ),
        ) {
            Ok(delta) => delta,
            Err(violation) => {
                violations.push(violation);
                complete = false;
                continue;
            }
        };
        let Some(runtime_nodes) = runtime_profile.get("nodes").and_then(Value::as_object) else {
            violations.push(format!(
                "native-fs repository baseline: runtime profile {} omitted nodes",
                profile.name
            ));
            complete = false;
            continue;
        };
        let Some(repository_nodes) = repository_profile.get("nodes").and_then(Value::as_object)
        else {
            violations.push(format!(
                "native-fs repository baseline: repository profile {} omitted nodes",
                profile.name
            ));
            complete = false;
            continue;
        };
        profile_deltas.insert(
            profile.name.to_owned(),
            Value::Object(Map::from_iter([
                (
                    "target".to_owned(),
                    Value::String(profile.target.to_owned()),
                ),
                ("roots".to_owned(), roots),
                ("full_package_keys".to_owned(), full_package_keys),
                (
                    "nodes".to_owned(),
                    object_delta(runtime_nodes, repository_nodes),
                ),
            ])),
        );
    }
    if !complete {
        return None;
    }

    Some(Value::Object(Map::from_iter([
        ("schema".to_owned(), Value::from(2)),
        (
            "runtime_baseline".to_owned(),
            Value::String(BASELINE_PATH.to_owned()),
        ),
        (
            "runtime_baseline_schema".to_owned(),
            Value::from(
                runtime_baseline
                    .get("schema")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
            ),
        ),
        (
            "review_basis".to_owned(),
            repository_review_basis(),
        ),
        (
            "metadata_semantics".to_owned(),
            Value::String(
                "cargo metadata --locked --filter-platform; repository-fs-candidate normal/build closure delta from the frozen runtime baseline"
                    .to_owned(),
            ),
        ),
        (
            "package_identities".to_owned(),
            object_delta(runtime_identities, &repository_identities),
        ),
        (
            "dependency_audit_profiles".to_owned(),
            Value::Object(profile_deltas),
        ),
        (
            "repository_operational_candidate_targets".to_owned(),
            string_array(REPOSITORY_OPERATIONAL_CANDIDATE_TARGETS),
        ),
        (
            "repository_fail_closed_targets".to_owned(),
            string_array(REPOSITORY_FAIL_CLOSED_TARGETS),
        ),
    ])))
}

fn object_delta(baseline: &Map<String, Value>, actual: &Map<String, Value>) -> Value {
    let mut added = Map::new();
    let mut changed = Map::new();
    let mut removed = Vec::new();
    for (key, value) in actual {
        match baseline.get(key) {
            None => {
                added.insert(key.clone(), value.clone());
            }
            Some(previous) if previous != value => {
                changed.insert(key.clone(), value.clone());
            }
            Some(_) => {}
        }
    }
    for key in baseline.keys() {
        if !actual.contains_key(key) {
            removed.push(Value::String(key.clone()));
        }
    }
    Value::Object(Map::from_iter([
        ("added".to_owned(), Value::Object(added)),
        ("changed".to_owned(), Value::Object(changed)),
        ("removed".to_owned(), Value::Array(removed)),
    ]))
}

fn value_string_set_delta(
    baseline: Option<&Value>,
    actual: Option<&Value>,
    owner: &str,
) -> Result<Value, String> {
    let baseline = baseline
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{owner}: runtime baseline omitted a string array"))?;
    let actual = actual
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{owner}: repository snapshot omitted a string array"))?;
    let baseline: BTreeSet<_> = sorted_strings(baseline)?.into_iter().collect();
    let actual: BTreeSet<_> = sorted_strings(actual)?.into_iter().collect();
    let added = actual
        .difference(&baseline)
        .cloned()
        .map(Value::String)
        .collect();
    let removed = baseline
        .difference(&actual)
        .cloned()
        .map(Value::String)
        .collect();
    Ok(Value::Object(Map::from_iter([
        ("added".to_owned(), Value::Array(added)),
        ("removed".to_owned(), Value::Array(removed)),
    ])))
}

fn repository_review_basis() -> Value {
    Value::Object(Map::from_iter([(
        "authority".to_owned(),
        Value::String("docs/research/u4c-repository-input-capability.md".to_owned()),
    )]))
}

fn review_basis() -> Value {
    Value::Object(Map::from_iter([
        (
            "authority".to_owned(),
            Value::String(
                "docs/research/native-filesystem-bound-handle-admission.md".to_owned(),
            ),
        ),
        (
            "windows_directory_share_mode".to_owned(),
            Value::Object(Map::from_iter([
                (
                    "package".to_owned(),
                    Value::String("cap-primitives@4.0.2".to_owned()),
                ),
                (
                    "identity_binding".to_owned(),
                    Value::String(
                        "package_identities.cap-primitives@4.0.2".to_owned(),
                    ),
                ),
                (
                    "reviewed_path".to_owned(),
                    Value::String("src/windows/fs/dir_utils.rs".to_owned()),
                ),
                (
                    "semantic_contract".to_owned(),
                    Value::String(
                        "root and descendant directory opens use FILE_FLAG_BACKUP_SEMANTICS and share only FILE_SHARE_READ | FILE_SHARE_WRITE, excluding FILE_SHARE_DELETE"
                            .to_owned(),
                    ),
                ),
            ])),
        ),
    ]))
}

fn build_dependency_audit_profile(
    kind: DependencyAuditKind,
    profile: DependencyAuditProfile,
    metadata: &Value,
    checksums: &BTreeMap<ChecksumKey, String>,
    identities: &mut BTreeMap<String, Value>,
    violations: &mut Vec<String>,
) -> Result<Value, String> {
    let document = MetadataDocument::parse(metadata, profile.target)?;
    let root = document.unique_package_id("baozi-io")?;
    let root_node = document.node(root)?;
    let full_reachable = document.production_closure(&[root.to_owned()])?;
    let profile_label = dependency_audit_profile_label(kind, profile.name);
    let mut full_package_keys = BTreeSet::new();
    for id in &full_reachable {
        if id == root {
            continue;
        }
        let package = document.package(id)?;
        let key = package_key(package)?;
        if !full_package_keys.insert(key.clone()) {
            return Err(format!(
                "{profile_label}: duplicate full-closure package key {key}"
            ));
        }
        if string_field(package, "source") != Some(CRATES_IO_SOURCE) {
            violations.push(format!(
                "{profile_label}: full-closure package {key} must come from {CRATES_IO_SOURCE}"
            ));
        }
    }
    let root_features = root_node
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{profile_label}: baozi-io omitted resolved features"))?;
    let actual_root_features = sorted_nondefault_features(root_features)?;
    let expected_root_features = match kind {
        DependencyAuditKind::Runtime => vec!["fs-candidate".to_owned()],
        DependencyAuditKind::Repository => {
            vec![
                "fs-candidate".to_owned(),
                "repository-fs-candidate".to_owned(),
            ]
        }
    };
    if actual_root_features != expected_root_features {
        violations.push(format!(
            "{profile_label}: baozi-io resolved features must be exactly {expected_root_features:?}, found {actual_root_features:?}"
        ));
    }

    let mut roots = Vec::new();
    for dependency in production_dependencies(root_node, profile.target)? {
        let package = document.package(&dependency.package)?;
        let name = required_string(package, "name", "package")?;
        if NATIVE_FILESYSTEM_PACKAGES.contains(&name) {
            roots.push(dependency.package);
        }
    }
    roots.sort_unstable();
    roots.dedup();

    let expected_names: BTreeSet<_> = match (kind, profile.direct_rustix) {
        (DependencyAuditKind::Runtime, true) => ["cap-std", "rustix"].into_iter().collect(),
        (DependencyAuditKind::Runtime, false) => ["cap-std"].into_iter().collect(),
        (DependencyAuditKind::Repository, true) => {
            ["cap-fs-ext", "cap-std", "rustix"].into_iter().collect()
        }
        (DependencyAuditKind::Repository, false) => ["cap-fs-ext", "cap-std"].into_iter().collect(),
    };
    let actual_names = roots
        .iter()
        .map(|id| {
            document
                .package(id)
                .and_then(|package| required_string(package, "name", "package"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if actual_names != expected_names {
        violations.push(format!(
            "{profile_label}: direct roots must be {expected_names:?}, found {actual_names:?}"
        ));
    }

    let reachable = document.production_closure(&roots)?;
    for id in &full_reachable {
        let package = document.package(id)?;
        let name = required_string(package, "name", "package")?;
        if NATIVE_FILESYSTEM_PACKAGES.contains(&name) && !reachable.contains(id) {
            violations.push(format!(
                "{profile_label}: native package {name} is reachable outside the admitted direct native dependency subgraph"
            ));
        }
    }
    let mut node_snapshots = Map::new();
    let mut profile_keys = BTreeSet::new();
    for id in &reachable {
        let package = document.package(id)?;
        let key = package_key(package)?;
        if !profile_keys.insert(key.clone()) {
            return Err(format!("{profile_label}: duplicate package key {key}"));
        }
        let identity = package_identity(package, checksums)?;
        if string_field(&identity, "source") != Some(CRATES_IO_SOURCE) {
            violations.push(format!(
                "{profile_label}: {key} must come from {CRATES_IO_SOURCE}"
            ));
        }
        if identity.get("checksum").and_then(Value::as_str).is_none() {
            violations.push(format!(
                "{profile_label}: {key} must have a crates.io checksum"
            ));
        }
        match identities.get(&key) {
            Some(existing) if existing != &identity => {
                violations.push(format!(
                    "{profile_label}: {key} has inconsistent package identity"
                ));
            }
            None => {
                identities.insert(key.clone(), identity);
            }
            _ => {}
        }

        let node = document.node(id)?;
        let features = node
            .get("features")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{profile_label}: {key} omitted features"))?;
        let features = sorted_strings(features)?;
        let mut dependencies = Vec::new();
        for dependency in production_dependencies(node, profile.target)? {
            if !reachable.contains(&dependency.package) {
                continue;
            }
            let child_key = package_key(document.package(&dependency.package)?)?;
            for kind in dependency.kinds {
                dependencies.push(format!("{kind}:{}={child_key}", dependency.name));
            }
        }
        dependencies.sort_unstable();
        dependencies.dedup();
        node_snapshots.insert(
            key,
            Value::Object(Map::from_iter([
                (
                    "features".to_owned(),
                    Value::Array(features.into_iter().map(Value::String).collect()),
                ),
                (
                    "dependencies".to_owned(),
                    Value::Array(dependencies.into_iter().map(Value::String).collect()),
                ),
            ])),
        );
    }

    check_critical_packages(
        kind,
        profile,
        &full_reachable,
        &document,
        checksums,
        violations,
    )?;

    let mut root_keys = roots
        .iter()
        .map(|id| document.package(id).and_then(package_key))
        .collect::<Result<Vec<_>, _>>()?;
    root_keys.sort_unstable();
    Ok(Value::Object(Map::from_iter([
        (
            "target".to_owned(),
            Value::String(profile.target.to_owned()),
        ),
        (
            "roots".to_owned(),
            Value::Array(root_keys.into_iter().map(Value::String).collect()),
        ),
        (
            "full_package_keys".to_owned(),
            Value::Array(full_package_keys.into_iter().map(Value::String).collect()),
        ),
        ("nodes".to_owned(), Value::Object(node_snapshots)),
    ])))
}

fn dependency_audit_profile_label(kind: DependencyAuditKind, profile: &str) -> String {
    match kind {
        DependencyAuditKind::Runtime => format!("native-fs profile {profile}"),
        DependencyAuditKind::Repository => format!("native-fs repository profile {profile}"),
    }
}

fn check_critical_packages(
    kind: DependencyAuditKind,
    profile: DependencyAuditProfile,
    reachable: &BTreeSet<String>,
    document: &MetadataDocument<'_>,
    checksums: &BTreeMap<ChecksumKey, String>,
    violations: &mut Vec<String>,
) -> Result<(), String> {
    let profile_label = dependency_audit_profile_label(kind, profile.name);
    let packages = reachable
        .iter()
        .map(|id| document.package(id))
        .collect::<Result<Vec<_>, _>>()?;

    if kind == DependencyAuditKind::Runtime
        && packages
            .iter()
            .any(|package| string_field(package, "name") == Some("cap-fs-ext"))
    {
        violations.push(format!(
            "{profile_label}: cap-fs-ext is forbidden in the resolved closure"
        ));
    }

    for expected in CRITICAL_PACKAGES {
        let matching: Vec<_> = packages
            .iter()
            .filter(|package| string_field(package, "name") == Some(expected.name))
            .collect();
        let required = match expected.name {
            "cap-fs-ext" => kind == DependencyAuditKind::Repository,
            "rustix" => profile.direct_rustix,
            _ => true,
        };
        if matching.is_empty() && !required {
            continue;
        }
        let package = match matching.as_slice() {
            [package] => **package,
            _ => {
                violations.push(format!(
                    "{profile_label}: expected exactly one {} package, found {}",
                    expected.name,
                    matching.len()
                ));
                continue;
            }
        };
        let checksum = package_checksum(package, checksums)?;
        let exact = string_field(package, "version") == Some(expected.version)
            && string_field(package, "source") == Some(CRATES_IO_SOURCE)
            && checksum == expected.checksum;
        if !exact {
            violations.push(format!(
                "{profile_label}: {} must resolve to {} from {CRATES_IO_SOURCE} with checksum {}",
                expected.name, expected.version, expected.checksum
            ));
        }
        if expected.name == "cap-fs-ext" && kind == DependencyAuditKind::Repository {
            let id = required_string(package, "id", "package")?;
            let node = document.node(id)?;
            let features = node
                .get("features")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{profile_label}: cap-fs-ext omitted resolved features"))?;
            let actual: BTreeSet<_> = sorted_strings(features)?.into_iter().collect();
            let expected = BTreeSet::from(["cap-std".to_owned(), "std".to_owned()]);
            if actual != expected {
                violations.push(format!(
                    "{profile_label}: cap-fs-ext resolved features must be exactly {expected:?}, found {actual:?}"
                ));
            }
        }
    }
    Ok(())
}

fn check_portable_profile(target: &str, metadata: &Value) -> Result<(), String> {
    let document = MetadataDocument::parse(metadata, target)?;
    let root = document.unique_package_id("baozi")?;
    let reachable = document.production_closure(&[root.to_owned()])?;
    let mut forbidden_packages = BTreeSet::new();
    let mut forbidden_features = BTreeSet::new();
    for id in reachable {
        let package = document.package(&id)?;
        let name = required_string(package, "name", "package")?;
        if NATIVE_FILESYSTEM_PACKAGES.contains(&name) {
            forbidden_packages.insert(name.to_owned());
        }

        let node = document.node(&id)?;
        let features = node
            .get("features")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                format!("native-fs portable profile {target}: {name} omitted resolved features")
            })?;
        for feature in features {
            let feature = feature.as_str().ok_or_else(|| {
                format!(
                    "native-fs portable profile {target}: {name} has a non-string resolved feature"
                )
            })?;
            if (name == "baozi" && matches!(feature, "native-fs" | "native-fs-candidate"))
                || (name == "baozi-io"
                    && matches!(feature, "fs" | "fs-candidate" | "repository-fs-candidate"))
            {
                forbidden_features.insert(format!("{name}/{feature}"));
            }
        }
    }
    if forbidden_packages.is_empty() && forbidden_features.is_empty() {
        Ok(())
    } else if forbidden_features.is_empty() {
        Err(format!(
            "native-fs portable profile {target}: byte-only graph contains forbidden packages {forbidden_packages:?}"
        ))
    } else if forbidden_packages.is_empty() {
        Err(format!(
            "native-fs portable profile {target}: byte-only graph enables forbidden features {forbidden_features:?}"
        ))
    } else {
        Err(format!(
            "native-fs portable profile {target}: byte-only graph contains forbidden packages {forbidden_packages:?} and enables forbidden features {forbidden_features:?}"
        ))
    }
}

struct MetadataDocument<'a> {
    target: &'a str,
    packages: BTreeMap<String, &'a Value>,
    nodes: BTreeMap<String, &'a Value>,
}

impl<'a> MetadataDocument<'a> {
    fn parse(metadata: &'a Value, target: &'a str) -> Result<Self, String> {
        let package_values = metadata
            .get("packages")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("native-fs profile {target}: metadata omitted packages"))?;
        let node_values = metadata
            .get("resolve")
            .and_then(|resolve| resolve.get("nodes"))
            .and_then(Value::as_array)
            .ok_or_else(|| format!("native-fs profile {target}: metadata omitted resolve.nodes"))?;
        let packages = unique_values_by_id(package_values, target, "package")?;
        let nodes = unique_values_by_id(node_values, target, "resolve node")?;
        Ok(Self {
            target,
            packages,
            nodes,
        })
    }

    fn unique_package_id(&self, name: &str) -> Result<&str, String> {
        let matching: Vec<_> = self
            .packages
            .iter()
            .filter(|(_, package)| string_field(package, "name") == Some(name))
            .map(|(id, _)| id.as_str())
            .collect();
        match matching.as_slice() {
            [id] => Ok(id),
            _ => Err(format!(
                "native-fs profile {}: expected one {name} package, found {}",
                self.target,
                matching.len()
            )),
        }
    }

    fn package(&self, id: &str) -> Result<&Value, String> {
        self.packages.get(id).copied().ok_or_else(|| {
            format!(
                "native-fs profile {}: resolve references unknown package {id}",
                self.target
            )
        })
    }

    fn node(&self, id: &str) -> Result<&Value, String> {
        self.nodes.get(id).copied().ok_or_else(|| {
            format!(
                "native-fs profile {}: missing resolve node for {id}",
                self.target
            )
        })
    }

    fn production_closure(&self, roots: &[String]) -> Result<BTreeSet<String>, String> {
        let mut pending: VecDeque<_> = roots.iter().cloned().collect();
        let mut reachable = BTreeSet::new();
        while let Some(id) = pending.pop_front() {
            if !reachable.insert(id.clone()) {
                continue;
            }
            let node = self.node(&id)?;
            for dependency in production_dependencies(node, self.target)? {
                pending.push_back(dependency.package);
            }
        }
        Ok(reachable)
    }
}

struct ProductionDependency {
    name: String,
    package: String,
    kinds: Vec<&'static str>,
}

fn production_dependencies(
    node: &Value,
    target: &str,
) -> Result<Vec<ProductionDependency>, String> {
    let dependencies = node
        .get("deps")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("native-fs profile {target}: resolve node omitted deps"))?;
    let mut result = Vec::new();
    for dependency in dependencies {
        let name = required_string(dependency, "name", "resolved dependency")?.to_owned();
        let package = required_string(dependency, "pkg", "resolved dependency")?.to_owned();
        let dep_kinds = dependency
            .get("dep_kinds")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                format!("native-fs profile {target}: dependency {name} omitted dep_kinds")
            })?;
        let mut kinds = BTreeSet::new();
        for dep_kind in dep_kinds {
            match dep_kind.get("kind") {
                None | Some(Value::Null) => {
                    kinds.insert("normal");
                }
                Some(Value::String(kind)) if kind == "build" => {
                    kinds.insert("build");
                }
                Some(Value::String(kind)) if kind == "dev" => {}
                Some(Value::String(kind)) => {
                    return Err(format!(
                        "native-fs profile {target}: dependency {name} has unknown kind {kind}"
                    ));
                }
                Some(_) => {
                    return Err(format!(
                        "native-fs profile {target}: dependency {name} has non-string kind"
                    ));
                }
            }
        }
        if !kinds.is_empty() {
            result.push(ProductionDependency {
                name,
                package,
                kinds: kinds.into_iter().collect(),
            });
        }
    }
    Ok(result)
}

fn unique_values_by_id<'a>(
    values: &'a [Value],
    target: &str,
    owner: &str,
) -> Result<BTreeMap<String, &'a Value>, String> {
    let mut result = BTreeMap::new();
    for value in values {
        let id = required_string(value, "id", owner)?.to_owned();
        if result.insert(id.clone(), value).is_some() {
            return Err(format!(
                "native-fs profile {target}: duplicate {owner} id {id}"
            ));
        }
    }
    Ok(result)
}

fn package_key(package: &Value) -> Result<String, String> {
    let name = required_string(package, "name", "package")?;
    let version = required_string(package, "version", "package")?;
    Ok(format!("{name}@{version}"))
}

fn package_identity(
    package: &Value,
    checksums: &BTreeMap<ChecksumKey, String>,
) -> Result<Value, String> {
    let source = required_string(package, "source", "package")?;
    let checksum = package_checksum(package, checksums)?;
    Ok(Value::Object(Map::from_iter([
        ("source".to_owned(), Value::String(source.to_owned())),
        ("checksum".to_owned(), Value::String(checksum.to_owned())),
    ])))
}

fn package_checksum<'a>(
    package: &Value,
    checksums: &'a BTreeMap<ChecksumKey, String>,
) -> Result<&'a str, String> {
    let name = required_string(package, "name", "package")?;
    let version = required_string(package, "version", "package")?;
    let source = required_string(package, "source", "package")?;
    checksums
        .get(&(name.to_owned(), version.to_owned(), source.to_owned()))
        .map(String::as_str)
        .ok_or_else(|| {
            format!("native-fs lockfile: missing checksum for {name}@{version} from {source}")
        })
}

fn check_baseline_shape(baseline: &Value, violations: &mut Vec<String>) {
    if baseline.get("schema").and_then(Value::as_u64) != Some(3) {
        violations.push("native-fs baseline: schema must be 3".to_owned());
    }
    let expected_profiles: BTreeSet<_> = DEPENDENCY_AUDIT_PROFILES
        .iter()
        .map(|profile| profile.name)
        .collect();
    let actual_profiles = baseline
        .get("dependency_audit_profiles")
        .and_then(Value::as_object)
        .map(|profiles| profiles.keys().map(String::as_str).collect::<BTreeSet<_>>());
    if actual_profiles.as_ref() != Some(&expected_profiles) {
        violations.push(format!(
            "native-fs baseline: dependency audit profile set must be {expected_profiles:?}, found {actual_profiles:?}"
        ));
    }
    check_target_array(
        baseline,
        "runtime_candidate_targets",
        RUNTIME_CANDIDATE_TARGETS,
        "runtime candidate",
        violations,
    );
    check_target_array(
        baseline,
        "release_registered_targets",
        RELEASE_REGISTERED_TARGETS,
        "release registered",
        violations,
    );
    check_baseline_target_relations(baseline, violations);
    let expected_portable: Vec<_> = PORTABLE_TARGETS
        .iter()
        .map(|target| (*target).to_owned())
        .collect();
    let actual_portable = baseline
        .get("portable_byte_targets")
        .and_then(Value::as_array)
        .and_then(|values| sorted_strings(values).ok());
    if actual_portable.as_ref() != Some(&expected_portable) {
        violations.push(format!(
            "native-fs baseline: portable targets must be {expected_portable:?}, found {actual_portable:?}"
        ));
    }
}

fn check_repository_baseline_shape(baseline: &Value, violations: &mut Vec<String>) {
    check_repository_target_partition(violations);
    if baseline.get("schema").and_then(Value::as_u64) != Some(2) {
        violations.push("native-fs repository baseline: schema must be 2".to_owned());
    }
    if string_field(baseline, "runtime_baseline") != Some(BASELINE_PATH) {
        violations.push(format!(
            "native-fs repository baseline: runtime_baseline must be {BASELINE_PATH:?}"
        ));
    }
    if baseline
        .get("runtime_baseline_schema")
        .and_then(Value::as_u64)
        != Some(3)
    {
        violations
            .push("native-fs repository baseline: runtime_baseline_schema must be 3".to_owned());
    }

    let expected_profiles: BTreeSet<_> = DEPENDENCY_AUDIT_PROFILES
        .iter()
        .map(|profile| profile.name)
        .collect();
    let actual_profiles = baseline
        .get("dependency_audit_profiles")
        .and_then(Value::as_object)
        .map(|profiles| profiles.keys().map(String::as_str).collect::<BTreeSet<_>>());
    if actual_profiles.as_ref() != Some(&expected_profiles) {
        violations.push(format!(
            "native-fs repository baseline: dependency audit profile set must be {expected_profiles:?}, found {actual_profiles:?}"
        ));
    }
    if let Some(profiles) = baseline
        .get("dependency_audit_profiles")
        .and_then(Value::as_object)
    {
        for profile in DEPENDENCY_AUDIT_PROFILES {
            let actual_target = profiles
                .get(profile.name)
                .and_then(|value| string_field(value, "target"));
            if actual_target != Some(profile.target) {
                violations.push(format!(
                    "native-fs repository baseline: profile {} target must be {:?}, found {actual_target:?}",
                    profile.name, profile.target
                ));
            }
        }
    }

    let mut expected_targets: Vec<_> = REPOSITORY_OPERATIONAL_CANDIDATE_TARGETS
        .iter()
        .map(|target| (*target).to_owned())
        .collect();
    expected_targets.sort_unstable();
    let actual_targets = baseline
        .get("repository_operational_candidate_targets")
        .and_then(Value::as_array)
        .and_then(|values| sorted_strings(values).ok());
    if actual_targets.as_ref() != Some(&expected_targets) {
        violations.push(format!(
            "native-fs repository baseline: repository operational candidate targets must be {expected_targets:?}, found {actual_targets:?}"
        ));
    }
    if let Some(values) = baseline
        .get("repository_operational_candidate_targets")
        .and_then(Value::as_array)
    {
        let mut unique = BTreeSet::new();
        for value in values {
            if let Some(target) = value.as_str()
                && !unique.insert(target)
            {
                violations.push(format!(
                    "native-fs repository baseline: repository operational candidate targets contain duplicate {target:?}"
                ));
            }
        }
    }
    let mut expected_fail_closed: Vec<_> = REPOSITORY_FAIL_CLOSED_TARGETS
        .iter()
        .map(|target| (*target).to_owned())
        .collect();
    expected_fail_closed.sort_unstable();
    let actual_fail_closed = baseline
        .get("repository_fail_closed_targets")
        .and_then(Value::as_array)
        .and_then(|values| sorted_strings(values).ok());
    if actual_fail_closed.as_ref() != Some(&expected_fail_closed) {
        violations.push(format!(
            "native-fs repository baseline: repository fail-closed targets must be {expected_fail_closed:?}, found {actual_fail_closed:?}"
        ));
    }
}

fn check_baseline_target_relations(baseline: &Value, violations: &mut Vec<String>) {
    let runtime_candidates = unique_target_set(
        baseline.get("runtime_candidate_targets"),
        "runtime candidate",
        violations,
    );
    let release_registered = unique_target_set(
        baseline.get("release_registered_targets"),
        "release registered",
        violations,
    );
    let dependency_audit = dependency_audit_target_set(baseline, violations);

    if let (Some(release_registered), Some(runtime_candidates)) =
        (&release_registered, &runtime_candidates)
    {
        let outside_candidates: Vec<_> = release_registered
            .difference(runtime_candidates)
            .cloned()
            .collect();
        if !outside_candidates.is_empty() {
            violations.push(format!(
                "native-fs baseline: release registered targets must be a subset of runtime candidate targets; outside candidates: {outside_candidates:?}"
            ));
        }
    }
    if let (Some(runtime_candidates), Some(dependency_audit)) =
        (&runtime_candidates, &dependency_audit)
    {
        let unaudited_candidates: Vec<_> = runtime_candidates
            .difference(dependency_audit)
            .cloned()
            .collect();
        if !unaudited_candidates.is_empty() {
            violations.push(format!(
                "native-fs baseline: runtime candidate targets must be a subset of dependency audit targets; unaudited candidates: {unaudited_candidates:?}"
            ));
        }
    }
}

fn unique_target_set(
    value: Option<&Value>,
    label: &str,
    violations: &mut Vec<String>,
) -> Option<BTreeSet<String>> {
    let values = value.and_then(Value::as_array)?;
    let mut targets = BTreeSet::new();
    for value in values {
        let target = value.as_str()?;
        if !targets.insert(target.to_owned()) {
            violations.push(format!(
                "native-fs baseline: {label} targets contain duplicate {target:?}"
            ));
        }
    }
    Some(targets)
}

fn dependency_audit_target_set(
    baseline: &Value,
    violations: &mut Vec<String>,
) -> Option<BTreeSet<String>> {
    let profiles = baseline
        .get("dependency_audit_profiles")
        .and_then(Value::as_object)?;
    let mut targets = BTreeSet::new();
    for (name, profile) in profiles {
        let Some(target) = string_field(profile, "target") else {
            violations.push(format!(
                "native-fs baseline: dependency audit profile {name:?} omitted its target"
            ));
            continue;
        };
        if !targets.insert(target.to_owned()) {
            violations.push(format!(
                "native-fs baseline: dependency audit profiles contain duplicate target {target:?}"
            ));
        }
    }
    Some(targets)
}

fn check_target_array(
    baseline: &Value,
    field: &str,
    expected: &[&str],
    label: &str,
    violations: &mut Vec<String>,
) {
    let mut expected: Vec<_> = expected.iter().map(|target| (*target).to_owned()).collect();
    expected.sort_unstable();
    let actual = baseline
        .get(field)
        .and_then(Value::as_array)
        .and_then(|values| sorted_strings(values).ok());
    if actual.as_ref() != Some(&expected) {
        violations.push(format!(
            "native-fs baseline: {label} targets must be {expected:?}, found {actual:?}"
        ));
    }
}

fn string_array(values: &[&str]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|value| Value::String((*value).to_owned()))
            .collect(),
    )
}

fn collect_differences(
    expected: &Value,
    actual: &Value,
    path: &str,
    difference_start: usize,
    violations: &mut Vec<String>,
) {
    if violations.len().saturating_sub(difference_start) >= MAX_BASELINE_DIFFERENCES {
        return;
    }
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            let keys: BTreeSet<_> = expected.keys().chain(actual.keys()).collect();
            for key in keys {
                match (expected.get(key), actual.get(key)) {
                    (Some(expected), Some(actual)) => collect_differences(
                        expected,
                        actual,
                        &format!("{path}.{key}"),
                        difference_start,
                        violations,
                    ),
                    (Some(_), None) => violations.push(format!(
                        "native-fs resolved closure: {path}.{key} is missing from Cargo metadata"
                    )),
                    (None, Some(_)) => violations.push(format!(
                        "native-fs resolved closure: {path}.{key} is not admitted by the baseline"
                    )),
                    (None, None) => {}
                }
                if violations.len().saturating_sub(difference_start) >= MAX_BASELINE_DIFFERENCES {
                    break;
                }
            }
        }
        (Value::Array(expected), Value::Array(actual)) if expected != actual => {
            violations.push(format!(
                "native-fs resolved closure: {path} expected {}, found {}",
                describe_value(&Value::Array(expected.clone())),
                describe_value(&Value::Array(actual.clone()))
            ))
        }
        _ if expected != actual => violations.push(format!(
            "native-fs resolved closure: {path} expected {}, found {}",
            describe_value(expected),
            describe_value(actual)
        )),
        _ => {}
    }
}

fn describe_value(value: &Value) -> String {
    let rendered = value.to_string();
    const LIMIT: usize = 512;
    if rendered.chars().count() <= LIMIT {
        rendered
    } else {
        let prefix: String = rendered.chars().take(LIMIT).collect();
        format!("{prefix}...")
    }
}

fn find_unique_package<'a>(
    metadata: &'a Value,
    name: &str,
    owner: &str,
    violations: &mut Vec<String>,
) -> Option<&'a Value> {
    let Some(packages) = metadata.get("packages").and_then(Value::as_array) else {
        violations.push(format!("native-fs {owner}: metadata omitted packages"));
        return None;
    };
    let matching: Vec<_> = packages
        .iter()
        .filter(|package| string_field(package, "name") == Some(name))
        .collect();
    match matching.as_slice() {
        [package] => Some(*package),
        _ => {
            violations.push(format!(
                "native-fs {owner}: expected exactly one {name} package, found {}",
                matching.len()
            ));
            None
        }
    }
}

fn sorted_strings(values: &[Value]) -> Result<Vec<String>, String> {
    let mut strings = values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "expected a string array".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    strings.sort_unstable();
    Ok(strings)
}

fn sorted_nondefault_features(values: &[Value]) -> Result<Vec<String>, String> {
    let mut features = sorted_strings(values)?;
    features.retain(|feature| feature != "default");
    Ok(features)
}

fn string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

fn required_string<'a>(value: &'a Value, field: &str, owner: &str) -> Result<&'a str, String> {
    string_field(value, field).ok_or_else(|| format!("{owner} omitted string field {field}"))
}
