//! Cargo-metadata checks for the production dependency direction contract.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

const OPERATION_PACKAGE: &str = "baozi-operation";
const ALLOWED_DIRECT_CONSUMERS: &[&str] =
    &["baozi-core", "baozi-import", "baozi-postprocess", "baozi"];
const REQUIRED_DIRECT_CONSUMERS: &[&str] = &["baozi-core", "baozi-import", "baozi-postprocess"];
const REQUIRED_GRAPH_PACKAGES: &[&str] =
    &["baozi-core", "baozi-import", "baozi-postprocess", "baozi"];
const NON_PRODUCTION_PACKAGES: &[&str] = &["baozi-test-support", "baozi-xtask"];

/// The result of checking production dependency direction and operation-crate placement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateGraphReport {
    /// Number of workspace packages represented by Cargo metadata.
    pub packages_checked: usize,
    /// Stable, human-readable contract violations.
    pub violations: Vec<String>,
}

/// Checks every production dependency direction plus operation's required direct consumers.
pub fn check_repository(repository_root: &Path) -> CrateGraphReport {
    let metadata = match load_metadata(repository_root) {
        Ok(metadata) => metadata,
        Err(violation) => {
            return CrateGraphReport {
                packages_checked: 0,
                violations: vec![violation],
            };
        }
    };
    let packages = match parse_workspace_packages(&metadata) {
        Ok(packages) => packages,
        Err(violation) => {
            return CrateGraphReport {
                packages_checked: 0,
                violations: vec![violation],
            };
        }
    };

    let mut violations = Vec::new();
    if !packages.contains_key(OPERATION_PACKAGE) {
        return CrateGraphReport {
            packages_checked: packages.len(),
            violations: vec!["workspace is missing baozi-operation".to_owned()],
        };
    }

    for package_name in REQUIRED_GRAPH_PACKAGES {
        if !packages.contains_key(*package_name) {
            violations.push(format!("workspace is missing {package_name}"));
        }
    }
    for package_name in packages
        .keys()
        .filter(|name| is_production_package(name) && !is_classified_production_package(name))
    {
        violations.push(format!(
            "{package_name} has no production dependency-direction classification"
        ));
    }

    check_dependency_directions(&packages, &mut violations);

    let allowed_direct_consumers: BTreeSet<_> = ALLOWED_DIRECT_CONSUMERS.iter().copied().collect();
    for package in packages.values() {
        let operation_dependencies: Vec<_> = package
            .dependencies
            .iter()
            .filter(|dependency| {
                dependency.name == OPERATION_PACKAGE && dependency.kind.as_deref() != Some("dev")
            })
            .collect();
        if operation_dependencies.is_empty() {
            continue;
        }
        if !allowed_direct_consumers.contains(package.name.as_str()) {
            violations.push(format!(
                "{} must not depend directly on baozi-operation",
                package.name
            ));
        } else if operation_dependencies.len() != 1
            || !is_canonical_operation_dependency(operation_dependencies[0])
        {
            violations.push(format!(
                "{} must have exactly one non-optional unconditional production dependency on baozi-operation",
                package.name
            ));
        }
    }

    for package_name in REQUIRED_DIRECT_CONSUMERS {
        let Some(package) = packages.get(*package_name) else {
            continue;
        };
        if !package
            .dependencies
            .iter()
            .any(is_canonical_operation_dependency)
        {
            violations.push(format!(
                "{package_name} must depend directly on baozi-operation"
            ));
        }
    }

    for source in ["baozi-core", "baozi-postprocess"] {
        if let Some(path) = production_path(&packages, source, "baozi-import") {
            violations.push(format!(
                "{source} must not reach baozi-import through production dependencies: {}",
                path.join(" -> ")
            ));
        }
    }

    if let Some(cycle) = production_cycle(&packages) {
        violations.push(format!(
            "production dependency cycle detected: {}",
            cycle.join(" -> ")
        ));
    }

    violations.sort_unstable();
    violations.dedup();
    CrateGraphReport {
        packages_checked: packages.len(),
        violations,
    }
}

#[derive(Debug)]
struct Package {
    name: String,
    dependencies: Vec<Dependency>,
}

#[derive(Debug)]
struct Dependency {
    name: String,
    optional: bool,
    kind: Option<String>,
    target: Option<String>,
}

fn load_metadata(repository_root: &Path) -> Result<Value, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--locked",
            "--format-version",
            "1",
            "--no-deps",
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

fn parse_workspace_packages(metadata: &Value) -> Result<BTreeMap<String, Package>, String> {
    let workspace_members = metadata
        .get("workspace_members")
        .and_then(Value::as_array)
        .ok_or_else(|| "cargo metadata omitted workspace_members".to_owned())?
        .iter()
        .map(|member| {
            member
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "cargo metadata contained a non-string workspace member".to_owned())
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let package_values = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "cargo metadata omitted packages".to_owned())?;

    let mut packages = BTreeMap::new();
    for package in package_values {
        let id = required_string(package, "id", "package")?;
        if !workspace_members.contains(id) {
            continue;
        }
        let name = required_string(package, "name", "package")?.to_owned();
        let dependencies = package
            .get("dependencies")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("cargo metadata omitted dependencies for {name}"))?
            .iter()
            .map(parse_dependency)
            .collect::<Result<Vec<_>, _>>()?;
        if packages
            .insert(name.clone(), Package { name, dependencies })
            .is_some()
        {
            return Err("cargo metadata contained duplicate workspace package names".to_owned());
        }
    }
    Ok(packages)
}

fn parse_dependency(value: &Value) -> Result<Dependency, String> {
    let name = required_string(value, "name", "dependency")?.to_owned();
    let optional = value
        .get("optional")
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("cargo metadata omitted optional for dependency {name}"))?;
    let kind = optional_string(value, "kind", &name)?;
    let target = optional_string(value, "target", &name)?;
    Ok(Dependency {
        name,
        optional,
        kind,
        target,
    })
}

fn optional_string(value: &Value, field: &str, owner: &str) -> Result<Option<String>, String> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!(
            "cargo metadata contained a non-string {field} for dependency {owner}"
        )),
    }
}

fn required_string<'a>(value: &'a Value, field: &str, owner: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("cargo metadata omitted {field} for {owner}"))
}

fn is_baozi_package(name: &str) -> bool {
    name == "baozi" || name.starts_with("baozi-")
}

fn is_production_package(name: &str) -> bool {
    is_baozi_package(name) && !NON_PRODUCTION_PACKAGES.contains(&name)
}

fn is_classified_production_package(name: &str) -> bool {
    matches!(
        name,
        "baozi"
            | "baozi-operation"
            | "baozi-core"
            | "baozi-io"
            | "baozi-import"
            | "baozi-postprocess"
    ) || name.starts_with("baozi-format-")
}

fn check_dependency_directions(packages: &BTreeMap<String, Package>, violations: &mut Vec<String>) {
    for package in packages
        .values()
        .filter(|package| is_production_package(&package.name))
    {
        for dependency in package
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind.as_deref() != Some("dev"))
            .filter(|dependency| is_baozi_package(&dependency.name))
        {
            if NON_PRODUCTION_PACKAGES.contains(&dependency.name.as_str()) {
                violations.push(format!(
                    "{} must not depend on non-production workspace package {} through production dependencies",
                    package.name, dependency.name
                ));
                continue;
            }
            if allowed_production_dependency(&package.name, &dependency.name) {
                continue;
            }
            if package.name == OPERATION_PACKAGE {
                violations.push(format!(
                    "baozi-operation must not depend on {}",
                    dependency.name
                ));
            } else if dependency.name == OPERATION_PACKAGE {
                violations.push(format!(
                    "{} must not depend directly on baozi-operation",
                    package.name
                ));
            } else {
                violations.push(format!(
                    "{} must not depend on {} through production dependencies",
                    package.name, dependency.name
                ));
            }
        }
    }
}

fn allowed_production_dependency(source: &str, target: &str) -> bool {
    // The maintained decoder is an excluded vendor package, private to this adapter.
    if source == "baozi-format-gltf" && target == "baozi-gltf" {
        return true;
    }
    match source {
        "baozi-operation" | "baozi-io" => false,
        "baozi-core" => target == "baozi-operation",
        "baozi-import" => matches!(target, "baozi-core" | "baozi-io" | "baozi-operation"),
        "baozi-postprocess" => matches!(target, "baozi-core" | "baozi-operation"),
        "baozi" => {
            matches!(
                target,
                "baozi-core"
                    | "baozi-io"
                    | "baozi-import"
                    | "baozi-operation"
                    | "baozi-postprocess"
            ) || target.starts_with("baozi-format-")
        }
        source if source.starts_with("baozi-format-") => {
            matches!(target, "baozi-core" | "baozi-import" | "baozi-io")
        }
        _ => false,
    }
}

fn is_canonical_operation_dependency(dependency: &Dependency) -> bool {
    dependency.name == OPERATION_PACKAGE
        && !dependency.optional
        && dependency.kind.is_none()
        && dependency.target.is_none()
}

fn production_path(
    packages: &BTreeMap<String, Package>,
    source: &str,
    target: &str,
) -> Option<Vec<String>> {
    if !packages.contains_key(source) || !packages.contains_key(target) {
        return None;
    }

    let mut pending = std::collections::VecDeque::from([source.to_owned()]);
    let mut predecessors = BTreeMap::<String, String>::new();
    let mut visited = BTreeSet::from([source.to_owned()]);
    while let Some(current) = pending.pop_front() {
        let package = packages.get(&current)?;
        let neighbors: BTreeSet<_> = package
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind.as_deref() != Some("dev"))
            .filter(|dependency| {
                packages.contains_key(&dependency.name) && is_production_package(&dependency.name)
            })
            .map(|dependency| dependency.name.clone())
            .collect();
        for neighbor in neighbors {
            if !visited.insert(neighbor.clone()) {
                continue;
            }
            predecessors.insert(neighbor.clone(), current.clone());
            if neighbor == target {
                let mut path = vec![target.to_owned()];
                let mut cursor = target;
                while let Some(predecessor) = predecessors.get(cursor) {
                    path.push(predecessor.clone());
                    cursor = predecessor;
                }
                path.reverse();
                return Some(path);
            }
            pending.push_back(neighbor);
        }
    }
    None
}

fn production_cycle(packages: &BTreeMap<String, Package>) -> Option<Vec<String>> {
    for source in packages.keys().filter(|name| is_production_package(name)) {
        let package = packages.get(source)?;
        let neighbors: BTreeSet<_> = package
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind.as_deref() != Some("dev"))
            .filter(|dependency| {
                packages.contains_key(&dependency.name) && is_production_package(&dependency.name)
            })
            .map(|dependency| dependency.name.clone())
            .collect();
        for neighbor in neighbors {
            if neighbor == *source {
                return Some(vec![source.clone(), source.clone()]);
            }
            if let Some(path) = production_path(packages, &neighbor, source) {
                let mut cycle = vec![source.clone()];
                cycle.extend(path);
                return Some(cycle);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_cycle_detects_indirect_cycles() {
        let packages = BTreeMap::from([
            package("baozi-core", [("baozi-postprocess", None)]),
            package("baozi-postprocess", [("baozi-core", None)]),
        ]);

        assert_eq!(
            production_cycle(&packages),
            Some(vec![
                "baozi-core".to_owned(),
                "baozi-postprocess".to_owned(),
                "baozi-core".to_owned(),
            ])
        );
    }

    #[test]
    fn production_cycle_excludes_dev_only_edges() {
        let packages = BTreeMap::from([
            package("baozi-core", [("baozi-operation", None)]),
            package("baozi-operation", [("baozi-core", Some("dev"))]),
        ]);

        assert_eq!(production_cycle(&packages), None);
    }

    fn package<const N: usize>(
        name: &str,
        dependencies: [(&str, Option<&str>); N],
    ) -> (String, Package) {
        let name = name.to_owned();
        let dependencies = dependencies
            .into_iter()
            .map(|(dependency, kind)| Dependency {
                name: dependency.to_owned(),
                optional: false,
                kind: kind.map(str::to_owned),
                target: None,
            })
            .collect();
        (name.clone(), Package { name, dependencies })
    }
}
