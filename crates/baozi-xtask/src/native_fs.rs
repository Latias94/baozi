//! Target-filtered dependency admission for the native filesystem adapter.

use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;

mod contract;

use contract::*;
pub use contract::{NativeFsMetadataProfiles, NativeFsReport};

/// Checks the portable root workspace and the standalone native candidate workspace.
///
/// Compiler target checks and native behavior tests cover adapter availability and behavior;
/// this check does not inspect Rust sources or promote candidate targets to release support.
/// Every Cargo query is locked. Registry checksums come from the candidate's own lockfile;
/// the reviewed local backend records its upstream archive checksum only as provenance.
pub fn check_repository(repository_root: &Path) -> NativeFsReport {
    let result = (|| {
        let baseline = read_json(&repository_root.join(BASELINE_PATH), "native-fs baseline")?;
        let repository_baseline = read_json(
            &repository_root.join(REPOSITORY_BASELINE_PATH),
            "native-fs repository baseline",
        )?;
        let lock_path = repository_root.join(CANDIDATE_LOCK_PATH);
        let lockfile = fs::read_to_string(&lock_path).map_err(|error| {
            format!("native-fs lockfile: failed to read {lock_path:?}: {error}")
        })?;
        let root = load_metadata(
            repository_root,
            "Cargo.toml",
            XTASK_RESOLUTION_TARGET,
            true,
            None,
        )?;
        let mut runtime = BTreeMap::new();
        let mut repository = BTreeMap::new();
        let mut portable = BTreeMap::new();
        for profile in DEPENDENCY_AUDIT_PROFILES {
            runtime.insert(
                profile.target.to_owned(),
                load_metadata(
                    repository_root,
                    CANDIDATE_MANIFEST_PATH,
                    profile.target,
                    false,
                    None,
                )?,
            );
            repository.insert(
                profile.target.to_owned(),
                load_metadata(
                    repository_root,
                    CANDIDATE_MANIFEST_PATH,
                    profile.target,
                    false,
                    Some("repository-capture"),
                )?,
            );
        }
        for target in PORTABLE_TARGETS {
            portable.insert(
                (*target).to_owned(),
                load_metadata(repository_root, "Cargo.toml", target, true, None)?,
            );
        }
        Ok(check_metadata_fixture(
            &lockfile,
            &baseline,
            &repository_baseline,
            NativeFsMetadataProfiles {
                root: &root,
                runtime: &runtime,
                repository: &repository,
                portable: &portable,
            },
        ))
    })();
    result.unwrap_or_else(|violation| NativeFsReport {
        profiles_checked: 0,
        violations: vec![violation],
    })
}

/// Checks already-loaded metadata without invoking Cargo or reading either workspace.
#[doc(hidden)]
pub fn check_metadata_fixture(
    candidate_lockfile: &str,
    baseline: &Value,
    repository_baseline: &Value,
    profiles: NativeFsMetadataProfiles<'_>,
) -> NativeFsReport {
    let mut violations = Vec::new();
    check_root_manifest(profiles.root, &mut violations);
    if let Some(candidate) = profiles.runtime.get(XTASK_RESOLUTION_TARGET) {
        check_candidate_manifest(profiles.root, candidate, &mut violations);
    }
    if let Err(violation) = check_xtask_profile(XTASK_RESOLUTION_TARGET, profiles.root) {
        violations.push(violation);
    }
    let checksums = parse_lockfile_checksums(candidate_lockfile);
    match checksums {
        Ok(checksums) => {
            if let Some(snapshot) = build_snapshot(
                profiles.runtime,
                profiles.portable,
                &checksums,
                &mut violations,
            ) {
                check_baseline_shape(baseline, &mut violations);
                let difference_start = violations.len();
                collect_differences(
                    baseline,
                    &snapshot,
                    "baseline",
                    difference_start,
                    &mut violations,
                );
            }
            if let Some(snapshot) = build_repository_delta_snapshot(
                baseline,
                profiles.repository,
                &checksums,
                &mut violations,
            ) {
                check_repository_baseline_shape(repository_baseline, &mut violations);
                let difference_start = violations.len();
                collect_differences(
                    repository_baseline,
                    &snapshot,
                    "repository baseline",
                    difference_start,
                    &mut violations,
                );
            }
        }
        Err(violation) => violations.push(violation),
    }
    violations.sort_unstable();
    violations.dedup();
    NativeFsReport {
        profiles_checked: profiles.runtime.len()
            + profiles.repository.len()
            + profiles.portable.len()
            + 1,
        violations,
    }
}

/// Builds the canonical resolved-closure document for fixture and maintenance tooling.
#[doc(hidden)]
pub fn resolved_closure_snapshot(
    runtime_metadata: &BTreeMap<String, Value>,
    portable_metadata: &BTreeMap<String, Value>,
    candidate_lockfile: &str,
) -> Result<Value, Vec<String>> {
    let mut violations = Vec::new();
    let checksums = match parse_lockfile_checksums(candidate_lockfile) {
        Ok(checksums) => checksums,
        Err(violation) => {
            return Err(vec![violation]);
        }
    };
    let snapshot = build_snapshot(
        runtime_metadata,
        portable_metadata,
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
    target: &str,
    all_features: bool,
    features: Option<&str>,
) -> Result<Value, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(cargo);
    command
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--manifest-path",
        ])
        .arg(repository_root.join(manifest_path))
        .args(["--filter-platform", target])
        .arg(if all_features {
            "--all-features"
        } else {
            "--no-default-features"
        })
        .current_dir(repository_root);
    if let Some(features) = features {
        command.args(["--features", features]);
    }
    let label = format!("native-fs metadata {manifest_path} {target} features={features:?}");
    let output = command
        .output()
        .map_err(|error| format!("{label}: failed to run cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{label}: cargo metadata --locked failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("{label}: cargo returned invalid JSON: {error}"))
}

fn read_json(path: &Path, label: &str) -> Result<Value, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("{label}: failed to read {path:?}: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{label}: invalid JSON: {error}"))
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

fn check_root_manifest(metadata: &Value, violations: &mut Vec<String>) {
    check_portable_declarations(metadata, violations);
    for (name, relative_manifest) in [
        ("baozi", "crates/baozi/Cargo.toml"),
        ("baozi-io", "crates/baozi-io/Cargo.toml"),
        (XTASK_PACKAGE, "crates/baozi-xtask/Cargo.toml"),
    ] {
        let Some(package) = find_unique_package(metadata, name, "root manifest", violations) else {
            continue;
        };
        let Some(id) = string_field(package, "id") else {
            violations.push(format!("native-fs root manifest: {name} omitted id"));
            continue;
        };
        if let Err(violation) = check_workspace_package(
            metadata,
            package,
            id,
            name,
            relative_manifest,
            "native-fs root manifest",
        ) {
            violations.push(violation);
        }
    }
}

fn check_portable_declarations(metadata: &Value, violations: &mut Vec<String>) {
    let Some(packages) = metadata.get("packages").and_then(Value::as_array) else {
        violations.push("native-fs manifest: metadata omitted packages".to_owned());
        return;
    };
    for package in packages
        .iter()
        .filter(|package| package.get("source") == Some(&Value::Null))
    {
        let Some(owner) = string_field(package, "name") else {
            violations.push("native-fs manifest: local package omitted name".to_owned());
            continue;
        };
        if owner == CANDIDATE_PACKAGE {
            violations.push(format!("native-fs root manifest: {CANDIDATE_PACKAGE} must stay outside the portable workspace"));
        }
        let Some(features) = package.get("features").and_then(Value::as_object) else {
            violations.push(format!(
                "native-fs manifest: {owner} omitted its feature table"
            ));
            continue;
        };
        if owner == XTASK_PACKAGE && !features.is_empty() {
            violations.push("native-fs manifest: baozi-xtask must remain featureless".to_owned());
        }
        for feature in features.keys() {
            if retired_native_feature(owner, feature) {
                violations.push(format!(
                    "native-fs manifest: {owner}/{feature} must be removed without an alias"
                ));
            }
        }
        let Some(dependencies) = package.get("dependencies").and_then(Value::as_array) else {
            violations.push(format!("native-fs manifest: {owner} omitted dependencies"));
            continue;
        };
        // Inspect all declarations, including inactive optional, target, build, and dev edges.
        for dependency in dependencies {
            let Some(name) = string_field(dependency, "name") else {
                violations.push(format!(
                    "native-fs manifest: {owner} has a dependency without a name"
                ));
                continue;
            };
            if name == CANDIDATE_PACKAGE || NATIVE_FILESYSTEM_PACKAGES.contains(&name) {
                violations.push(format!("native-fs manifest: {owner} must not depend on {name}; native dependencies belong to the standalone candidate"));
            }
            if owner == XTASK_PACKAGE && name == "baozi-io" {
                violations
                    .push("native-fs manifest: baozi-xtask must not depend on baozi-io".to_owned());
            }
            let key = string_field(dependency, "rename").unwrap_or(name);
            if let Some(requests) = dependency.get("features").and_then(Value::as_array) {
                for request in requests.iter().filter_map(Value::as_str) {
                    if retired_native_feature(name, request) {
                        violations.push(format!(
                            "native-fs manifest: {owner} must not request removed {name}/{request}"
                        ));
                    }
                }
            }
            for (feature, values) in features {
                for value in values
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    if let Some((forwarded_key, request)) = value.split_once('/')
                        && forwarded_key.trim_end_matches('?') == key
                        && retired_native_feature(name, request)
                    {
                        violations.push(format!("native-fs manifest: {owner}/{feature} must not forward removed {name}/{request}"));
                    }
                }
            }
        }
    }
}

fn retired_native_feature(package: &str, feature: &str) -> bool {
    match package {
        "baozi-io" => matches!(feature, "fs" | "fs-candidate" | "repository-fs-candidate"),
        "baozi" => matches!(feature, "native-fs" | "native-fs-candidate"),
        XTASK_PACKAGE => feature == "repository-input-candidate",
        _ => false,
    }
}

fn check_candidate_manifest(root: &Value, metadata: &Value, violations: &mut Vec<String>) {
    let Some(package) = find_unique_package(
        metadata,
        CANDIDATE_PACKAGE,
        "candidate manifest",
        violations,
    ) else {
        return;
    };
    let identity = (|| {
        let id = required_string(package, "id", CANDIDATE_PACKAGE)?;
        check_candidate_workspace(metadata, package, id, "native-fs candidate manifest")?;
        let repository_root = required_string(root, "workspace_root", "root metadata")?;
        let expected_root = Path::new(repository_root).join("candidates/baozi-native-io");
        let actual_root = required_string(metadata, "workspace_root", "candidate metadata")?;
        if Path::new(actual_root) != expected_root {
            return Err(format!(
                "native-fs candidate manifest: workspace_root must be {}, found {actual_root}",
                expected_root.display()
            ));
        }
        Ok(())
    })();
    if let Err(violation) = identity {
        violations.push(violation);
    }
    if package.get("publish") != Some(&Value::Array(Vec::new())) {
        violations.push(format!(
            "native-fs manifest: {CANDIDATE_PACKAGE} must set publish = false"
        ));
    }
    let expected_features =
        serde_json::json!({"default": [], "repository-capture": ["dep:cap-fs-ext"]});
    if package.get("features") != Some(&expected_features) {
        violations.push(format!(
            "native-fs manifest: {CANDIDATE_PACKAGE} features must be exactly {expected_features}"
        ));
    }
    let Some(dependencies) = package.get("dependencies").and_then(Value::as_array) else {
        violations.push(format!(
            "native-fs manifest: {CANDIDATE_PACKAGE} omitted dependencies"
        ));
        return;
    };
    let actual = dependencies
        .iter()
        .filter(|dependency| string_field(dependency, "kind") != Some("dev"))
        .filter_map(|dependency| string_field(dependency, "name"))
        .collect::<Vec<_>>();
    let actual_set = actual.iter().copied().collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        "baozi-io",
        "thiserror",
        "cap-std",
        "cap-primitives",
        "rustix",
        "cap-fs-ext",
    ]);
    if actual_set != expected || actual.len() != expected.len() {
        violations.push(format!("native-fs manifest: {CANDIDATE_PACKAGE} production dependencies must be exactly {expected:?} without duplicates, found {actual:?}"));
    }
    for expected in [
        ManifestDependency {
            name: "cap-std",
            requirement: "=4.0.3",
            path: None,
            target: CAP_STD_TARGET,
            optional: false,
            features: &[],
        },
        ManifestDependency {
            name: "rustix",
            requirement: "=1.1.4",
            path: None,
            target: RUSTIX_TARGET,
            optional: false,
            features: &["fs", "std"],
        },
        ManifestDependency {
            name: "cap-fs-ext",
            requirement: "=4.0.3",
            path: None,
            target: CAP_STD_TARGET,
            optional: true,
            features: &["std"],
        },
    ] {
        check_manifest_dependency(dependencies, expected, violations);
    }
    let io = dependencies
        .iter()
        .find(|dependency| string_field(dependency, "name") == Some("baozi-io"));
    let repository_root = string_field(root, "workspace_root").unwrap_or("");
    let expected_io_path = Path::new(repository_root).join("crates/baozi-io");
    let valid_io = io.is_some_and(|dependency| {
        dependency.get("source") == Some(&Value::Null)
            && string_field(dependency, "path")
                .is_some_and(|path| Path::new(path) == expected_io_path)
            && dependency.get("kind") == Some(&Value::Null)
            && dependency.get("rename") == Some(&Value::Null)
            && dependency.get("target") == Some(&Value::Null)
            && dependency.get("optional") == Some(&Value::Bool(false))
            && dependency.get("uses_default_features") == Some(&Value::Bool(false))
            && dependency.get("features") == Some(&serde_json::json!([]))
    });
    if !valid_io {
        violations.push(format!("native-fs manifest: {CANDIDATE_PACKAGE} must depend unconditionally on portable baozi-io at {} without features", expected_io_path.display()));
    }
}

fn check_candidate_workspace(
    metadata: &Value,
    package: &Value,
    id: &str,
    label: &str,
) -> Result<(), String> {
    let workspace_root = required_string(metadata, "workspace_root", "candidate metadata")?;
    if !Path::new(workspace_root).ends_with("candidates/baozi-native-io") {
        return Err(format!(
            "{label}: workspace_root must be the standalone candidates/baozi-native-io directory"
        ));
    }
    check_workspace_package(
        metadata,
        package,
        id,
        CANDIDATE_PACKAGE,
        "Cargo.toml",
        label,
    )?;
    if metadata.get("workspace_members") != Some(&serde_json::json!([id])) {
        return Err(format!(
            "{label}: candidate must be the only member of its own workspace"
        ));
    }
    if metadata
        .get("resolve")
        .and_then(|resolve| string_field(resolve, "root"))
        != Some(id)
    {
        return Err(format!(
            "{label}: candidate must be the resolve root of its own workspace"
        ));
    }
    Ok(())
}

fn check_xtask_profile(target: &str, metadata: &Value) -> Result<(), String> {
    let label = format!("native-fs xtask profile {target}");
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
    if document.node(xtask_id)?.get("features") != Some(&serde_json::json!([])) {
        return Err(format!("{label}: xtask must resolve no features"));
    }
    let reachable = document.production_closure(&[xtask_id.to_owned()])?;
    for id in reachable {
        let name = required_string(document.package(&id)?, "name", "package")?;
        if name == "baozi-io"
            || name == CANDIDATE_PACKAGE
            || NATIVE_FILESYSTEM_PACKAGES.contains(&name)
        {
            return Err(format!("{label}: xtask must not reach {name}"));
        }
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
    path: Option<&'a Path>,
    target: &'a str,
    features: &'a [&'a str],
    optional: bool,
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

    let (source_matches, location) = match expected.path {
        Some(path) => (
            dependency.get("source") == Some(&Value::Null)
                && string_field(dependency, "path").is_some_and(|actual| Path::new(actual) == path),
            format!("path={}", path.display()),
        ),
        None => (
            string_field(dependency, "source") == Some(CRATES_IO_SOURCE)
                && dependency.get("path").is_none_or(Value::is_null),
            format!("source={CRATES_IO_SOURCE}"),
        ),
    };
    let exact = source_matches
        && string_field(dependency, "req") == Some(expected.requirement)
        && dependency.get("kind") == Some(&Value::Null)
        && dependency.get("rename") == Some(&Value::Null)
        && dependency.get("optional").and_then(Value::as_bool) == Some(expected.optional)
        && dependency
            .get("uses_default_features")
            .and_then(Value::as_bool)
            == Some(false)
        && string_field(dependency, "target") == Some(expected.target)
        && actual_features.as_ref() == Some(&expected_features);
    if !exact {
        violations.push(format!(
            "native-fs manifest: direct {} edge drifted from {location}, req={}, optional={}, default-features=false, features={expected_features:?}, target={}",
            expected.name, expected.requirement, expected.optional, expected.target
        ));
    }
}

fn build_snapshot(
    target_metadata: &BTreeMap<String, Value>,
    portable_metadata: &BTreeMap<String, Value>,
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
        let Some(metadata) = portable_metadata.get(*target) else {
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
        ("schema".to_owned(), Value::from(RUNTIME_BASELINE_SCHEMA)),
        ("review_basis".to_owned(), review_basis()),
        (
            "metadata_semantics".to_owned(),
            Value::String(
                "cargo metadata --locked --filter-platform; normal/build closure only".to_owned(),
            ),
        ),
        (
            "manifest_path".to_owned(),
            Value::String(CANDIDATE_MANIFEST_PATH.to_owned()),
        ),
        (
            "lockfile_path".to_owned(),
            Value::String(CANDIDATE_LOCK_PATH.to_owned()),
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
    if runtime_baseline.get("schema").and_then(Value::as_u64) != Some(RUNTIME_BASELINE_SCHEMA) {
        violations.push(format!(
            "native-fs repository baseline: runtime baseline schema must be {RUNTIME_BASELINE_SCHEMA}"
        ));
        return None;
    }
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
        ("schema".to_owned(), Value::from(REPOSITORY_BASELINE_SCHEMA)),
        (
            "runtime_baseline".to_owned(),
            Value::String(BASELINE_PATH.to_owned()),
        ),
        (
            "runtime_baseline_schema".to_owned(),
            Value::from(RUNTIME_BASELINE_SCHEMA),
        ),
        (
            "review_basis".to_owned(),
            repository_review_basis(),
        ),
        (
            "metadata_semantics".to_owned(),
            Value::String(
                "cargo metadata --locked --filter-platform; repository-capture normal/build closure delta from the frozen runtime baseline"
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
                    Value::String("cap-primitives@4.0.3".to_owned()),
                ),
                (
                    "identity_binding".to_owned(),
                    Value::String(
                        "package_identities.cap-primitives@4.0.3".to_owned(),
                    ),
                ),
                (
                    "source".to_owned(),
                    Value::String(CAP_PRIMITIVES_SOURCE.to_owned()),
                ),
                (
                    "patches".to_owned(),
                    Value::String(CAP_PRIMITIVES_PATCHES_PATH.to_owned()),
                ),
                (
                    "reviewed_path".to_owned(),
                    Value::String(format!("{CAP_PRIMITIVES_PATH}/src/windows/fs/dir_utils.rs")),
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
    let profile_label = dependency_audit_profile_label(kind, profile.name);
    let root = document.unique_package_id(CANDIDATE_PACKAGE)?;
    check_candidate_workspace(metadata, document.package(root)?, root, &profile_label)?;
    let workspace_root = Path::new(required_string(
        metadata,
        "workspace_root",
        "candidate metadata",
    )?);
    let repository_root = workspace_root
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| format!("{profile_label}: candidate workspace omitted repository parent"))?;
    let cap_primitives_path = repository_root.join(CAP_PRIMITIVES_PATH);
    let declarations = document
        .package(root)?
        .get("dependencies")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{profile_label}: {CANDIDATE_PACKAGE} omitted dependencies"))?;
    check_manifest_dependency(
        declarations,
        ManifestDependency {
            name: "cap-primitives",
            requirement: "=4.0.3",
            path: Some(&cap_primitives_path),
            target: CAP_STD_TARGET,
            optional: false,
            features: &[],
        },
        violations,
    );
    let expected_io_manifest = repository_root.join("crates/baozi-io/Cargo.toml");
    let io_id = document.unique_package_id("baozi-io")?;
    let io_package = document.package(io_id)?;
    if io_package.get("source") != Some(&Value::Null)
        || string_field(io_package, "manifest_path")
            .is_none_or(|path| Path::new(path) != expected_io_manifest)
    {
        return Err(format!(
            "{profile_label}: baozi-io must be the portable path package at {}",
            expected_io_manifest.display()
        ));
    }
    let io_features = document
        .node(io_id)?
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{profile_label}: baozi-io omitted resolved features"))?;
    if !sorted_nondefault_features(io_features)?.is_empty() {
        violations.push(format!(
            "{profile_label}: portable baozi-io must resolve no nondefault features"
        ));
    }
    let root_node = document.node(root)?;
    let full_reachable = document.production_closure(&[root.to_owned()])?;
    let root_dependencies = production_dependencies(root_node, profile.target)?;
    if !root_dependencies
        .iter()
        .any(|dependency| dependency.package == io_id && dependency.kinds.contains(&"normal"))
    {
        violations.push(format!(
            "{profile_label}: candidate must depend directly on portable baozi-io"
        ));
    }
    if !root_dependencies.iter().any(|dependency| {
        dependency.name == "cap_primitives"
            && dependency.kinds == ["normal"]
            && document
                .package(&dependency.package)
                .is_ok_and(|package| string_field(package, "name") == Some("cap-primitives"))
    }) {
        violations.push(format!(
            "{profile_label}: candidate must depend directly on local cap-primitives through a normal cap_primitives edge"
        ));
    }
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
        if id != io_id
            && string_field(package, "name") != Some("cap-primitives")
            && string_field(package, "source") != Some(CRATES_IO_SOURCE)
        {
            violations.push(format!(
                "{profile_label}: full-closure package {key} must come from {CRATES_IO_SOURCE}"
            ));
        }
    }
    let root_features = root_node
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{profile_label}: {CANDIDATE_PACKAGE} omitted resolved features"))?;
    let actual_root_features = sorted_nondefault_features(root_features)?;
    let expected_root_features = match kind {
        DependencyAuditKind::Runtime => vec![],
        DependencyAuditKind::Repository => vec!["repository-capture".to_owned()],
    };
    if actual_root_features != expected_root_features {
        violations.push(format!(
            "{profile_label}: {CANDIDATE_PACKAGE} resolved features must be exactly {expected_root_features:?}, found {actual_root_features:?}"
        ));
    }

    let mut roots = Vec::new();
    for dependency in root_dependencies {
        let package = document.package(&dependency.package)?;
        let name = required_string(package, "name", "package")?;
        if NATIVE_FILESYSTEM_PACKAGES.contains(&name) {
            roots.push(dependency.package);
        }
    }
    roots.sort_unstable();
    roots.dedup();

    let expected_names: BTreeSet<_> = match (kind, profile.direct_rustix) {
        (DependencyAuditKind::Runtime, true) => ["cap-primitives", "cap-std", "rustix"]
            .into_iter()
            .collect(),
        (DependencyAuditKind::Runtime, false) => {
            ["cap-primitives", "cap-std"].into_iter().collect()
        }
        (DependencyAuditKind::Repository, true) => {
            ["cap-fs-ext", "cap-primitives", "cap-std", "rustix"]
                .into_iter()
                .collect()
        }
        (DependencyAuditKind::Repository, false) => ["cap-fs-ext", "cap-primitives", "cap-std"]
            .into_iter()
            .collect(),
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
        let identity = package_identity(package, checksums, &cap_primitives_path)?;
        if string_field(package, "name") != Some("cap-primitives") {
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

    // The sole local backend's version and manifest path are checked by package_identity.
    let local_count = packages
        .iter()
        .filter(|package| string_field(package, "name") == Some("cap-primitives"))
        .count();
    if local_count != 1 {
        violations.push(format!(
            "{profile_label}: expected exactly one cap-primitives package, found {local_count}"
        ));
    }

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
    let members = metadata
        .get("workspace_members")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!("native-fs portable profile {target}: metadata omitted workspace_members")
        })?;
    let roots = sorted_strings(members)?;
    if roots.is_empty()
        || !roots.iter().any(|id| {
            document
                .package(id)
                .is_ok_and(|package| string_field(package, "name") == Some("baozi"))
        })
    {
        return Err(format!(
            "native-fs portable profile {target}: expected the portable root workspace including baozi"
        ));
    }
    let reachable = document.production_closure(&roots)?;
    let mut forbidden_packages = BTreeSet::new();
    let mut forbidden_features = BTreeSet::new();
    for id in reachable {
        let name = required_string(document.package(&id)?, "name", "package")?;
        if name == CANDIDATE_PACKAGE || NATIVE_FILESYSTEM_PACKAGES.contains(&name) {
            forbidden_packages.insert(name.to_owned());
        }
        let features = document
            .node(&id)?
            .get("features")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                format!("native-fs portable profile {target}: {name} omitted resolved features")
            })?;
        for feature in sorted_strings(features)? {
            if retired_native_feature(name, &feature)
                || (name == XTASK_PACKAGE && !feature.is_empty())
            {
                forbidden_features.insert(format!("{name}/{feature}"));
            }
        }
    }
    if forbidden_packages.is_empty() && forbidden_features.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "native-fs portable profile {target}: portable workspace contains forbidden packages {forbidden_packages:?} and features {forbidden_features:?}"
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
    cap_primitives_path: &Path,
) -> Result<Value, String> {
    if string_field(package, "name") == Some("cap-primitives") {
        let expected_manifest = cap_primitives_path.join("Cargo.toml");
        if package.get("source") != Some(&Value::Null)
            || string_field(package, "version") != Some(CAP_PRIMITIVES_VERSION)
            || string_field(package, "manifest_path")
                .is_none_or(|path| Path::new(path) != expected_manifest)
        {
            return Err(format!(
                "native-fs package: cap-primitives must resolve to local cap-primitives@{CAP_PRIMITIVES_VERSION} with source=null and manifest_path={}",
                expected_manifest.display()
            ));
        }
        return Ok(serde_json::json!({
            "source": CAP_PRIMITIVES_SOURCE,
            "upstream_source": CRATES_IO_SOURCE,
            "upstream_version": CAP_PRIMITIVES_VERSION,
            "upstream_checksum": CAP_PRIMITIVES_UPSTREAM_CHECKSUM,
            "patches": CAP_PRIMITIVES_PATCHES_PATH,
        }));
    }
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
    if baseline.get("schema").and_then(Value::as_u64) != Some(RUNTIME_BASELINE_SCHEMA) {
        violations.push(format!(
            "native-fs baseline: schema must be {RUNTIME_BASELINE_SCHEMA}"
        ));
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

fn check_repository_baseline_shape(baseline: &Value, violations: &mut Vec<String>) {
    check_repository_target_partition(violations);
    if baseline.get("schema").and_then(Value::as_u64) != Some(REPOSITORY_BASELINE_SCHEMA) {
        violations.push(format!(
            "native-fs repository baseline: schema must be {REPOSITORY_BASELINE_SCHEMA}"
        ));
    }
    if string_field(baseline, "runtime_baseline") != Some(BASELINE_PATH) {
        violations.push(format!(
            "native-fs repository baseline: runtime_baseline must be {BASELINE_PATH:?}"
        ));
    }
    if baseline
        .get("runtime_baseline_schema")
        .and_then(Value::as_u64)
        != Some(RUNTIME_BASELINE_SCHEMA)
    {
        violations.push(format!(
            "native-fs repository baseline: runtime_baseline_schema must be {RUNTIME_BASELINE_SCHEMA}"
        ));
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

fn string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

fn string_set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
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
