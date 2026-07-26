use serde_json::Value;
use std::path::{Path, PathBuf};
use syn::ext::IdentExt;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Attribute, Item, ItemMod, Meta, Token, UseTree};

pub(super) fn identifier_name(identifier: &syn::Ident) -> String {
    identifier.unraw().to_string()
}

pub(super) fn identifier_is(identifier: &syn::Ident, expected: &str) -> bool {
    identifier.unraw() == expected
}

pub(super) fn path_is_identifier(path: &syn::Path, expected: &str) -> bool {
    path.segments.len() == 1
        && path
            .segments
            .first()
            .is_some_and(|segment| identifier_is(&segment.ident, expected))
}

pub(super) fn path_ends_in_identifier(path: &syn::Path, expected: &str) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| identifier_is(&segment.ident, expected))
}

pub(super) fn is_admitted_candidate_implementation_macro(path: &syn::Path) -> bool {
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

pub(super) fn token_identifier(token: &str) -> &str {
    token.strip_prefix("r#").unwrap_or(token)
}

pub(super) fn string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

pub(super) fn use_tree_mentions_identifier(tree: &UseTree, expected: &str) -> bool {
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

pub(super) fn resolve_module_file(module_directory: &Path, item: &ItemMod) -> Option<PathBuf> {
    let name = identifier_name(&item.ident);
    [
        module_directory.join(format!("{name}.rs")),
        module_directory.join(name).join("mod.rs"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

pub(super) fn module_source_override(attributes: &[Attribute]) -> Option<&'static str> {
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

pub(super) fn module_directory_for_file(source: &Path) -> PathBuf {
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

pub(super) fn display_source_path(repository_root: &Path, path: &Path) -> String {
    path.strip_prefix(repository_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub(super) fn item_attributes(item: &Item) -> &[Attribute] {
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

pub(super) fn parse_meta_arguments(list: &syn::MetaList) -> Option<Punctuated<Meta, Token![,]>> {
    Punctuated::<Meta, Token![,]>::parse_terminated
        .parse2(list.tokens.clone())
        .ok()
}
