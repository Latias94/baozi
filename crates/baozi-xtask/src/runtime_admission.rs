//! Runtime-neutrality and host-admission governance checks.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{
    Attribute, Expr, ExprCall, ExprMethodCall, Item, ItemMod, ItemStatic, Meta, Token, Type,
    UseTree,
};

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
const GLOBAL_ADMISSION_TYPES: &[&str] = &[
    "ExecutorPool",
    "Semaphore",
    "TaskPool",
    "ThreadPool",
    "WorkerPool",
];

/// The result of checking runtime neutrality and host-owned admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAdmissionReport {
    /// Number of production Baozi workspace packages checked.
    pub production_packages_checked: usize,
    /// Number of reachable production Rust source files checked.
    pub production_source_files_checked: usize,
    /// Stable, human-readable contract violations.
    pub violations: Vec<String>,
}

/// Checks production dependency reachability and explicit scheduling entry points.
pub fn check_repository(repository_root: &Path) -> RuntimeAdmissionReport {
    let metadata = match load_metadata(repository_root) {
        Ok(metadata) => metadata,
        Err(violation) => {
            return RuntimeAdmissionReport {
                production_packages_checked: 0,
                production_source_files_checked: 0,
                violations: vec![violation],
            };
        }
    };
    let graph = match MetadataGraph::parse(&metadata) {
        Ok(graph) => graph,
        Err(violation) => {
            return RuntimeAdmissionReport {
                production_packages_checked: 0,
                production_source_files_checked: 0,
                violations: vec![violation],
            };
        }
    };

    let production_roots = graph.production_workspace_packages();
    let mut violations = Vec::new();
    check_forbidden_dependencies(&graph, &production_roots, &mut violations);

    let mut source_files_checked = 0_usize;
    let mut visited_sources = BTreeSet::new();
    for package_id in &production_roots {
        let Some(package) = graph.packages.get(package_id) else {
            continue;
        };
        for source in &package.production_targets {
            let module_directory = source.parent().unwrap_or(repository_root);
            scan_source_file(
                repository_root,
                &package.name,
                source,
                module_directory,
                &mut visited_sources,
                &mut source_files_checked,
                &mut violations,
            );
        }
    }

    violations.sort_unstable();
    violations.dedup();
    RuntimeAdmissionReport {
        production_packages_checked: production_roots.len(),
        production_source_files_checked: source_files_checked,
        violations,
    }
}

fn load_metadata(repository_root: &Path) -> Result<Value, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let output = Command::new(cargo)
        .args([
            "metadata",
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

#[derive(Debug)]
struct MetadataGraph {
    packages: BTreeMap<String, Package>,
    production_edges: BTreeMap<String, Vec<String>>,
    workspace_members: BTreeSet<String>,
}

#[derive(Debug)]
struct Package {
    name: String,
    production_targets: Vec<PathBuf>,
}

impl MetadataGraph {
    fn parse(metadata: &Value) -> Result<Self, String> {
        let workspace_members = string_set(metadata, "workspace_members", "cargo metadata")?;
        let package_values = metadata
            .get("packages")
            .and_then(Value::as_array)
            .ok_or_else(|| "cargo metadata omitted packages".to_owned())?;
        let mut packages = BTreeMap::new();
        for package in package_values {
            let id = required_string(package, "id", "package")?.to_owned();
            let name = required_string(package, "name", "package")?.to_owned();
            let target_values = package
                .get("targets")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("cargo metadata omitted targets for {name}"))?;
            let mut production_targets = Vec::new();
            for target in target_values {
                let kinds = target
                    .get("kind")
                    .and_then(Value::as_array)
                    .ok_or_else(|| format!("cargo metadata omitted target kind for {name}"))?;
                if !kinds.iter().any(is_production_target_kind) {
                    continue;
                }
                let source = required_string(target, "src_path", "target")?;
                production_targets.push(PathBuf::from(source));
            }
            production_targets.sort_unstable();
            production_targets.dedup();
            packages.insert(
                id,
                Package {
                    name,
                    production_targets,
                },
            );
        }

        let resolve = metadata
            .get("resolve")
            .and_then(Value::as_object)
            .ok_or_else(|| "cargo metadata omitted resolve graph".to_owned())?;
        let nodes = resolve
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| "cargo metadata omitted resolve nodes".to_owned())?;
        let mut production_edges = BTreeMap::new();
        for node in nodes {
            let id = required_string(node, "id", "resolve node")?.to_owned();
            let dependencies = node
                .get("deps")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("cargo metadata omitted dependencies for {id}"))?;
            let mut edges = Vec::new();
            for dependency in dependencies {
                if dependency_has_production_kind(dependency)? {
                    edges.push(
                        required_string(dependency, "pkg", "resolved dependency")?.to_owned(),
                    );
                }
            }
            edges.sort_unstable();
            edges.dedup();
            production_edges.insert(id, edges);
        }

        Ok(Self {
            packages,
            production_edges,
            workspace_members,
        })
    }

    fn production_workspace_packages(&self) -> Vec<String> {
        let mut packages = self
            .workspace_members
            .iter()
            .filter(|id| {
                self.packages
                    .get(*id)
                    .is_some_and(|package| is_production_package(&package.name))
            })
            .cloned()
            .collect::<Vec<_>>();
        packages
            .sort_unstable_by(|left, right| self.package_name(left).cmp(self.package_name(right)));
        packages
    }

    fn package_name(&self, id: &str) -> &str {
        self.packages
            .get(id)
            .map_or("<unknown-package>", |package| package.name.as_str())
    }
}

fn string_set(document: &Value, field: &str, owner: &str) -> Result<BTreeSet<String>, String> {
    document
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{owner} omitted {field}"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{owner} contained a non-string {field} entry"))
        })
        .collect()
}

fn required_string<'a>(value: &'a Value, field: &str, owner: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("cargo metadata omitted {field} for {owner}"))
}

fn is_production_target_kind(value: &Value) -> bool {
    value.as_str().is_some_and(|kind| {
        matches!(
            kind,
            "bin" | "cdylib" | "dylib" | "lib" | "proc-macro" | "rlib" | "staticlib"
        )
    })
}

fn dependency_has_production_kind(dependency: &Value) -> Result<bool, String> {
    let kinds = dependency
        .get("dep_kinds")
        .and_then(Value::as_array)
        .ok_or_else(|| "cargo metadata omitted resolved dependency kinds".to_owned())?;
    Ok(kinds.iter().any(|kind| {
        matches!(kind.get("kind"), None | Some(Value::Null))
            || kind.get("kind").and_then(Value::as_str) == Some("build")
    }))
}

fn is_production_package(name: &str) -> bool {
    (name == "baozi" || name.starts_with("baozi-")) && !NON_PRODUCTION_PACKAGES.contains(&name)
}

fn check_forbidden_dependencies(
    graph: &MetadataGraph,
    production_roots: &[String],
    violations: &mut Vec<String>,
) {
    for root in production_roots {
        let mut pending = VecDeque::from([root.clone()]);
        let mut visited = BTreeSet::from([root.clone()]);
        let mut predecessors = BTreeMap::<String, String>::new();
        while let Some(current) = pending.pop_front() {
            let Some(dependencies) = graph.production_edges.get(&current) else {
                continue;
            };
            for dependency in dependencies {
                if !visited.insert(dependency.clone()) {
                    continue;
                }
                predecessors.insert(dependency.clone(), current.clone());
                if FORBIDDEN_RUNTIME_PACKAGES.contains(&graph.package_name(dependency)) {
                    let path = dependency_path(graph, root, dependency, &predecessors);
                    violations.push(format!(
                        "forbidden runtime dependency in production normal/build graph: {}",
                        path.join(" -> ")
                    ));
                }
                pending.push_back(dependency.clone());
            }
        }
    }
}

fn dependency_path(
    graph: &MetadataGraph,
    root: &str,
    target: &str,
    predecessors: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut path = vec![target.to_owned()];
    let mut cursor = target;
    while cursor != root {
        let Some(predecessor) = predecessors.get(cursor) else {
            break;
        };
        path.push(predecessor.clone());
        cursor = predecessor;
    }
    path.reverse();
    path.into_iter()
        .map(|id| graph.package_name(&id).to_owned())
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn scan_source_file(
    repository_root: &Path,
    package: &str,
    source: &Path,
    module_directory: &Path,
    visited: &mut BTreeSet<PathBuf>,
    files_checked: &mut usize,
    violations: &mut Vec<String>,
) {
    let canonical = match fs::canonicalize(source) {
        Ok(path) => path,
        Err(error) => {
            violations.push(format!(
                "cannot inspect production source {}: {error}",
                display_path(repository_root, source)
            ));
            return;
        }
    };
    if !visited.insert(canonical) {
        return;
    }
    *files_checked = files_checked.saturating_add(1);
    let text = match fs::read_to_string(source) {
        Ok(text) => text,
        Err(error) => {
            violations.push(format!(
                "cannot read production source {}: {error}",
                display_path(repository_root, source)
            ));
            return;
        }
    };
    let file = match syn::parse_file(&text) {
        Ok(file) => file,
        Err(error) => {
            violations.push(format!(
                "cannot parse production source {}: {error}",
                display_path(repository_root, source)
            ));
            return;
        }
    };
    scan_module_items(
        repository_root,
        package,
        source,
        module_directory,
        &file.items,
        visited,
        files_checked,
        violations,
    );
}

#[allow(clippy::too_many_arguments)]
fn scan_module_items(
    repository_root: &Path,
    package: &str,
    source: &Path,
    module_directory: &Path,
    items: &[Item],
    visited: &mut BTreeSet<PathBuf>,
    files_checked: &mut usize,
    violations: &mut Vec<String>,
) {
    let aliases = ThreadAliases::from_items(items);
    let source_label = display_path(repository_root, source);
    for item in items {
        if item_is_test_only(item) {
            continue;
        }
        if let Item::Mod(item_mod) = item {
            if let Some((_, nested)) = &item_mod.content {
                scan_module_items(
                    repository_root,
                    package,
                    source,
                    &module_directory.join(item_mod.ident.to_string()),
                    nested,
                    visited,
                    files_checked,
                    violations,
                );
            } else if let Some(module_file) =
                resolve_module_file(source, module_directory, item_mod)
            {
                let nested_directory = module_directory_for_file(&module_file);
                scan_source_file(
                    repository_root,
                    package,
                    &module_file,
                    &nested_directory,
                    visited,
                    files_checked,
                    violations,
                );
            } else {
                violations.push(format!(
                    "cannot resolve production module {} from {source_label}",
                    item_mod.ident
                ));
            }
            continue;
        }

        let mut scanner = RuntimeSourceVisitor {
            aliases: &aliases,
            package,
            source: &source_label,
            violations,
        };
        scanner.visit_item(item);
    }
}

fn resolve_module_file(source: &Path, module_directory: &Path, item: &ItemMod) -> Option<PathBuf> {
    if let Some(path) = path_attribute(&item.attrs) {
        return source.parent().map(|parent| parent.join(path));
    }
    let name = item.ident.to_string();
    [
        module_directory.join(format!("{name}.rs")),
        module_directory.join(name).join("mod.rs"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

fn path_attribute(attributes: &[Attribute]) -> Option<PathBuf> {
    attributes.iter().find_map(|attribute| {
        if !attribute.path().is_ident("path") {
            return None;
        }
        let Meta::NameValue(name_value) = &attribute.meta else {
            return None;
        };
        let Expr::Lit(literal) = &name_value.value else {
            return None;
        };
        let syn::Lit::Str(path) = &literal.lit else {
            return None;
        };
        Some(PathBuf::from(path.value()))
    })
}

fn module_directory_for_file(source: &Path) -> PathBuf {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    if source.file_name().and_then(|name| name.to_str()) == Some("mod.rs") {
        parent.to_path_buf()
    } else {
        source
            .file_stem()
            .map_or_else(|| parent.to_path_buf(), |stem| parent.join(stem))
    }
}

fn display_path(repository_root: &Path, path: &Path) -> String {
    path.strip_prefix(repository_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn item_is_test_only(item: &Item) -> bool {
    let attributes = item_attributes(item);
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("test")
            || attribute.path().is_ident("bench")
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
    if !attribute.path().is_ident("cfg") {
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
        Meta::Path(path) => path.is_ident("test"),
        Meta::List(list) if list.path.is_ident("all") => parse_meta_arguments(list)
            .is_some_and(|arguments| arguments.iter().any(meta_requires_test)),
        Meta::List(list) if list.path.is_ident("any") => {
            parse_meta_arguments(list).is_some_and(|arguments| {
                !arguments.is_empty() && arguments.iter().all(meta_requires_test)
            })
        }
        Meta::List(_) | Meta::NameValue(_) => false,
    }
}

fn parse_meta_arguments(list: &syn::MetaList) -> Option<Punctuated<Meta, Token![,]>> {
    Punctuated::<Meta, Token![,]>::parse_terminated
        .parse2(list.tokens.clone())
        .ok()
}

#[derive(Debug, Default)]
struct ThreadAliases {
    modules: BTreeSet<String>,
    functions: BTreeSet<String>,
    builders: BTreeSet<String>,
    thread_glob: bool,
}

impl ThreadAliases {
    fn from_items(items: &[Item]) -> Self {
        let mut aliases = Self::default();
        for item in items {
            let Item::Use(item_use) = item else {
                continue;
            };
            if item_is_test_only(item) {
                continue;
            }
            collect_use_tree(&item_use.tree, &mut Vec::new(), &mut aliases);
        }
        aliases
    }
}

fn collect_use_tree(tree: &UseTree, prefix: &mut Vec<String>, aliases: &mut ThreadAliases) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_tree(&path.tree, prefix, aliases);
            prefix.pop();
        }
        UseTree::Name(name) => {
            let mut path = prefix.clone();
            path.push(name.ident.to_string());
            register_use_alias(&path, name.ident.to_string(), aliases);
        }
        UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            path.push(rename.ident.to_string());
            register_use_alias(&path, rename.rename.to_string(), aliases);
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                collect_use_tree(tree, prefix, aliases);
            }
        }
        UseTree::Glob(_) => {
            if prefix.iter().map(String::as_str).eq(["std", "thread"]) {
                aliases.thread_glob = true;
            }
        }
    }
}

fn register_use_alias(path: &[String], alias: String, aliases: &mut ThreadAliases) {
    let path = path.iter().map(String::as_str).collect::<Vec<_>>();
    match path.as_slice() {
        ["std", "thread"] => {
            aliases.modules.insert(alias);
        }
        ["std", "thread", "self"] => {
            let module_alias = if alias == "self" {
                "thread".to_owned()
            } else {
                alias
            };
            aliases.modules.insert(module_alias);
        }
        ["std", "thread", "spawn" | "scope"] => {
            aliases.functions.insert(alias);
        }
        ["std", "thread", "Builder"] => {
            aliases.builders.insert(alias);
        }
        _ => {}
    }
}

struct RuntimeSourceVisitor<'a> {
    aliases: &'a ThreadAliases,
    package: &'a str,
    source: &'a str,
    violations: &'a mut Vec<String>,
}

impl RuntimeSourceVisitor<'_> {
    fn report_thread_entry(&mut self, entry: &str) {
        self.violations.push(format!(
            "production thread entry point {entry} in {} ({})",
            self.source, self.package
        ));
    }
}

impl<'ast> Visit<'ast> for RuntimeSourceVisitor<'_> {
    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        if let Expr::Path(function) = expression.func.as_ref()
            && let Some(entry) = thread_function_entry(&function.path, self.aliases)
        {
            self.report_thread_entry(entry);
        }
        syn::visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast ExprMethodCall) {
        if expression.method == "spawn"
            && expression_is_thread_builder(&expression.receiver, self.aliases)
        {
            self.report_thread_entry("std::thread::Builder::spawn");
        }
        syn::visit::visit_expr_method_call(self, expression);
    }

    fn visit_item_mod(&mut self, _item: &'ast ItemMod) {}

    fn visit_item_static(&mut self, item: &'ast ItemStatic) {
        if let Some(type_name) = global_admission_type(item.ty.as_ref()) {
            self.violations.push(format!(
                "global admission or worker-pool state {}: {type_name} in {} ({})",
                item.ident, self.source, self.package
            ));
        }
        syn::visit::visit_item_static(self, item);
    }
}

fn thread_function_entry(path: &syn::Path, aliases: &ThreadAliases) -> Option<&'static str> {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    match segments.as_slice() {
        [standard, thread, entry]
            if standard == "std"
                && thread == "thread"
                && matches!(entry.as_str(), "spawn" | "scope") =>
        {
            Some(if entry == "scope" {
                "std::thread::scope"
            } else {
                "std::thread::spawn"
            })
        }
        [module, entry]
            if aliases.modules.contains(module) && matches!(entry.as_str(), "spawn" | "scope") =>
        {
            Some(if entry == "scope" {
                "std::thread::scope"
            } else {
                "std::thread::spawn"
            })
        }
        [entry]
            if aliases.functions.contains(entry)
                || (aliases.thread_glob && matches!(entry.as_str(), "spawn" | "scope")) =>
        {
            Some(if entry == "scope" {
                "std::thread::scope"
            } else {
                "std::thread::spawn"
            })
        }
        [standard, thread, builder, entry]
            if standard == "std"
                && thread == "thread"
                && builder == "Builder"
                && entry == "spawn" =>
        {
            Some("std::thread::Builder::spawn")
        }
        _ => None,
    }
}

fn expression_is_thread_builder(expression: &Expr, aliases: &ThreadAliases) -> bool {
    match expression {
        Expr::Call(call) => match call.func.as_ref() {
            Expr::Path(function) => thread_builder_constructor(&function.path, aliases),
            _ => false,
        },
        Expr::Group(group) => expression_is_thread_builder(&group.expr, aliases),
        Expr::MethodCall(method) => expression_is_thread_builder(&method.receiver, aliases),
        Expr::Paren(parenthesized) => expression_is_thread_builder(&parenthesized.expr, aliases),
        Expr::Reference(reference) => expression_is_thread_builder(&reference.expr, aliases),
        _ => false,
    }
}

fn thread_builder_constructor(path: &syn::Path, aliases: &ThreadAliases) -> bool {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    match segments.as_slice() {
        [standard, thread, builder, constructor] => {
            standard == "std" && thread == "thread" && builder == "Builder" && constructor == "new"
        }
        [module, builder, constructor] => {
            aliases.modules.contains(module) && builder == "Builder" && constructor == "new"
        }
        [builder, constructor] => {
            (aliases.builders.contains(builder) || (aliases.thread_glob && builder == "Builder"))
                && constructor == "new"
        }
        _ => false,
    }
}

fn global_admission_type(value: &Type) -> Option<String> {
    let mut visitor = GlobalAdmissionTypeVisitor { found: None };
    visitor.visit_type(value);
    visitor.found
}

struct GlobalAdmissionTypeVisitor {
    found: Option<String>,
}

impl<'ast> Visit<'ast> for GlobalAdmissionTypeVisitor {
    fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
        let name = segment.ident.to_string();
        if self.found.is_none() && GLOBAL_ADMISSION_TYPES.contains(&name.as_str()) {
            self.found = Some(name);
            return;
        }
        syn::visit::visit_path_segment(self, segment);
    }
}
