//! Dependency checks for Baozi's runtime-neutral production graph.

use serde_json::Value;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

const NON_PRODUCTION_PACKAGES: &[&str] = &["baozi-test-support", "baozi-xtask"];
const FORBIDDEN_RUNTIME_PACKAGES: &[&str] = &[
    "async-executor",
    "async-global-executor",
    "async-std",
    "blocking",
    "futures-executor",
    "rayon",
    "rayon-core",
    "scheduled-thread-pool",
    "scoped_threadpool",
    "smol",
    "threadpool",
    "tokio",
];

/// The result of checking production dependencies for runtimes and schedulers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAdmissionReport {
    /// Number of production Baozi workspace packages checked.
    pub production_packages_checked: usize,
    /// Stable, human-readable dependency violations or metadata errors.
    pub violations: Vec<String>,
}

/// Checks normal/build dependency reachability using one locked, all-features metadata query.
pub fn check_repository(repository_root: &Path) -> RuntimeAdmissionReport {
    load_metadata(repository_root)
        .and_then(|metadata| check_metadata(&metadata))
        .unwrap_or_else(|violation| RuntimeAdmissionReport {
            production_packages_checked: 0,
            violations: vec![violation],
        })
}

/// Checks Cargo metadata without reading Rust sources or running additional commands.
///
/// All target conditions are included. Dev-only edges and repository tooling roots are excluded.
pub fn check_metadata(metadata: &Value) -> Result<RuntimeAdmissionReport, String> {
    let packages = required_array(metadata, "packages", "metadata")?
        .iter()
        .map(|package| {
            Ok((
                required_string(package, "id", "package")?,
                required_string(package, "name", "package")?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let mut roots = Vec::new();
    for member in required_array(metadata, "workspace_members", "metadata")? {
        let id = member
            .as_str()
            .ok_or_else(|| "cargo metadata contained a non-string workspace member".to_owned())?;
        let name = packages
            .get(id)
            .ok_or_else(|| format!("cargo metadata omitted workspace package {id}"))?;
        if (*name == "baozi" || name.starts_with("baozi-"))
            && !NON_PRODUCTION_PACKAGES.contains(name)
        {
            roots.push(id);
        }
    }
    roots.sort_unstable();
    roots.dedup();
    if roots.is_empty() {
        return Err("cargo metadata contained no production Baozi workspace packages".to_owned());
    }

    let resolve = metadata
        .get("resolve")
        .filter(|value| value.is_object())
        .ok_or_else(|| "cargo metadata omitted resolve graph".to_owned())?;
    let mut edges = BTreeMap::new();
    for node in required_array(resolve, "nodes", "resolve graph")? {
        let id = required_string(node, "id", "resolve node")?;
        if !packages.contains_key(id) {
            return Err(format!("cargo metadata omitted resolved package {id}"));
        }
        let mut dependencies = Vec::new();
        for dependency in required_array(node, "deps", id)? {
            let target = required_string(dependency, "pkg", "resolved dependency")?;
            if !packages.contains_key(target) {
                return Err(format!(
                    "cargo metadata omitted dependency package {target}"
                ));
            }
            if has_production_kind(dependency)? {
                dependencies.push(target);
            }
        }
        dependencies.sort_unstable();
        dependencies.dedup();
        edges.insert(id, dependencies);
    }
    for id in packages.keys() {
        if !edges.contains_key(id) {
            return Err(format!("cargo metadata omitted resolve node for {id}"));
        }
    }

    let mut violations = Vec::new();
    for &root in &roots {
        let mut predecessors = BTreeMap::from([(root, root)]);
        let mut pending = VecDeque::from([root]);
        while let Some(current) = pending.pop_front() {
            for &dependency in &edges[current] {
                let Entry::Vacant(entry) = predecessors.entry(dependency) else {
                    continue;
                };
                entry.insert(current);
                if FORBIDDEN_RUNTIME_PACKAGES.contains(&packages[dependency]) {
                    let mut path = vec![packages[dependency]];
                    let mut cursor = current;
                    loop {
                        path.push(packages[cursor]);
                        if cursor == root {
                            break;
                        }
                        cursor = predecessors[cursor];
                    }
                    path.reverse();
                    violations.push(format!(
                        "forbidden runtime dependency in production normal/build graph: {}",
                        path.join(" -> ")
                    ));
                }
                pending.push_back(dependency);
            }
        }
    }
    violations.sort_unstable();
    violations.dedup();
    Ok(RuntimeAdmissionReport {
        production_packages_checked: roots.len(),
        violations,
    })
}

fn load_metadata(repository_root: &Path) -> Result<Value, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--locked",
            "--format-version",
            "1",
            "--all-features",
            "--manifest-path",
        ])
        .arg(repository_root.join("Cargo.toml"))
        .current_dir(repository_root)
        .output()
        .map_err(|error| format!("failed to run cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("cargo metadata returned invalid JSON: {error}"))
}

fn required_array<'a>(value: &'a Value, field: &str, owner: &str) -> Result<&'a [Value], String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("cargo metadata omitted {field} for {owner}"))
}

fn required_string<'a>(value: &'a Value, field: &str, owner: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("cargo metadata omitted {field} for {owner}"))
}

fn has_production_kind(dependency: &Value) -> Result<bool, String> {
    let kinds = required_array(dependency, "dep_kinds", "resolved dependency")?;
    if kinds.is_empty() {
        return Err("cargo metadata contained an empty dependency kind list".to_owned());
    }
    let mut production = false;
    for kind in kinds {
        match kind.get("kind") {
            Some(Value::Null) => production = true,
            Some(Value::String(kind)) if kind == "build" => production = true,
            Some(Value::String(kind)) if kind == "dev" => {}
            _ => return Err("cargo metadata contained an invalid dependency kind".to_owned()),
        }
    }
    Ok(production)
}
