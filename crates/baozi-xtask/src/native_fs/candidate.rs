use super::contract::{
    CANDIDATE_TEST_ALLOWED_MACROS, CANDIDATE_TEST_MODULE_DIGEST, CANDIDATE_TEST_SOURCE_PATH,
    CandidateTestPlatform, REPOSITORY_CANDIDATE_SURFACE_PATH, REPOSITORY_CANDIDATE_TARGETS,
    REPOSITORY_CANDIDATE_TEST_MODULE_DIGEST, REPOSITORY_CANDIDATE_TEST_SOURCE_PATH,
    REPOSITORY_FAIL_CLOSED_TARGETS, REPOSITORY_OPERATIONAL_CANDIDATE_TARGETS,
    REPOSITORY_PUBLIC_EXPORTS, REQUIRED_CANDIDATE_TESTS, REQUIRED_REPOSITORY_CANDIDATE_TESTS,
    RUNTIME_CANDIDATE_TARGETS, RUNTIME_PUBLIC_EXPORTS, RequiredCandidateTest,
};
use super::support::{
    display_source_path, identifier_is, identifier_name,
    is_admitted_candidate_implementation_macro, item_attributes, module_directory_for_file,
    module_source_override, parse_meta_arguments, path_ends_in_identifier, path_is_identifier,
    resolve_module_file,
};
use proc_macro2::TokenStream;
use quote::ToTokens;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{Attribute, Item, ItemMod, Meta, Token, UseTree, Visibility};

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

pub(super) fn candidate_test_crate_ancestor_violations(
    repository_root: &Path,
    label: &str,
) -> Vec<String> {
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

pub(super) fn string_set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(super) fn check_repository_target_partition(violations: &mut Vec<String>) {
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

pub(super) fn check_candidate_implementation_cfg_sources(
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
