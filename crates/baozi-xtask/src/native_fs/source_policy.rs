use super::contract::{FACADE_PACKAGE, FORMAT_PACKAGE_PREFIX, IMPORT_PACKAGE};
use super::support::{
    display_source_path, identifier_is, identifier_name, item_attributes,
    module_directory_for_file, module_source_override, parse_meta_arguments,
    path_ends_in_identifier, path_is_identifier, resolve_module_file, string_field,
    token_identifier, use_tree_mentions_identifier,
};
use proc_macro2::{TokenStream, TokenTree};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{Attribute, Item, ItemMod, Meta, Token, UseTree};

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

pub(super) fn check_format_import_production_sources(
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

pub(super) fn scan_inline_production_items(
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

pub(super) fn default_std_aliases() -> BTreeSet<String> {
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
