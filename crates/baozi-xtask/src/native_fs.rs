//! Target-filtered dependency admission for the native filesystem adapter.

use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;

mod candidate;
mod contract;
mod source_policy;
mod support;

pub use candidate::{
    candidate_production_source_violations, candidate_test_source_violations,
    repository_candidate_surface_violations, repository_candidate_test_source_violations,
    repository_candidate_test_tree_violations, runtime_candidate_surface_violations,
};
use candidate::{
    candidate_test_crate_ancestor_violations, check_candidate_implementation_cfg_sources,
    check_repository_target_partition, string_set,
};
use contract::*;
pub use contract::{NativeFsReport, RepositoryMetadataProfiles};
use source_policy::check_format_import_production_sources;
pub use source_policy::production_source_text_violations;
use support::string_field;

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

fn required_string<'a>(value: &'a Value, field: &str, owner: &str) -> Result<&'a str, String> {
    string_field(value, field).ok_or_else(|| format!("{owner} omitted string field {field}"))
}
