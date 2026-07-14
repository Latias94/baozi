//! First-party format ownership-protocol source checks.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

const FORMAT_CRATE_PREFIX: &str = "baozi-format-";
const FORBIDDEN_OWNERSHIP_MARKERS: &[&str] = &[
    "LiveStorage",
    "HeldReservation",
    "ImportLiveReservation",
    "MeteredStorage",
    "reserve_live",
    "resize_live",
    "adopt_reservation",
    ".adopt(",
    "builder.add_node(",
    "builder.add_child_node(",
    "builder.add_mesh(",
    "builder.add_material(",
    "builder.add_texture(",
    "builder.add_animation(",
    "builder.add_camera(",
    "builder.add_light(",
    "builder.add_skin(",
];

/// The result of checking every first-party format Rust source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatOwnershipReport {
    /// Number of Rust source files below first-party format `src` directories.
    pub format_source_files_checked: usize,
    /// Stable, human-readable contract violations.
    pub violations: Vec<String>,
}

/// Checks all `crates/baozi-format-*/src/**/*.rs` files, including unlinked modules.
pub fn check_repository(repository_root: &Path) -> FormatOwnershipReport {
    let crates_directory = repository_root.join("crates");
    let mut violations = Vec::new();
    let mut source_files = Vec::new();

    let crate_paths = match sorted_directory_entries(&crates_directory) {
        Ok(paths) => paths,
        Err(error) => {
            return FormatOwnershipReport {
                format_source_files_checked: 0,
                violations: vec![format!(
                    "first-party format ownership: cannot inspect {}: {error}",
                    display_path(repository_root, &crates_directory)
                )],
            };
        }
    };

    for crate_path in crate_paths {
        let Some(crate_name) = crate_path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if !crate_name.starts_with(FORMAT_CRATE_PREFIX) {
            continue;
        }

        let metadata = match fs::symlink_metadata(&crate_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                violations.push(format!(
                    "first-party format ownership: cannot inspect {}: {error}",
                    display_path(repository_root, &crate_path)
                ));
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            violations.push(format!(
                "first-party format ownership: cannot verify symlink {}",
                display_path(repository_root, &crate_path)
            ));
            continue;
        }
        if metadata.is_dir() {
            collect_rust_sources(
                repository_root,
                &crate_path.join("src"),
                &mut source_files,
                &mut violations,
            );
        }
    }

    source_files.sort_unstable();
    source_files.dedup();
    for source_file in &source_files {
        let source = match fs::read_to_string(source_file) {
            Ok(source) => source,
            Err(error) => {
                violations.push(format!(
                    "first-party format ownership: cannot read {}: {error}",
                    display_path(repository_root, source_file)
                ));
                continue;
            }
        };
        for marker in FORBIDDEN_OWNERSHIP_MARKERS {
            if source.contains(marker) {
                violations.push(format!(
                    "first-party format ownership: forbidden protocol {marker:?} in {}",
                    display_path(repository_root, source_file)
                ));
            }
        }
    }

    violations.sort_unstable();
    violations.dedup();
    FormatOwnershipReport {
        format_source_files_checked: source_files.len(),
        violations,
    }
}

fn collect_rust_sources(
    repository_root: &Path,
    directory: &Path,
    source_files: &mut Vec<PathBuf>,
    violations: &mut Vec<String>,
) {
    let entries = match sorted_directory_entries(directory) {
        Ok(entries) => entries,
        Err(error) => {
            violations.push(format!(
                "first-party format ownership: cannot inspect {}: {error}",
                display_path(repository_root, directory)
            ));
            return;
        }
    };

    for path in entries {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                violations.push(format!(
                    "first-party format ownership: cannot inspect {}: {error}",
                    display_path(repository_root, &path)
                ));
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            violations.push(format!(
                "first-party format ownership: cannot verify symlink {}",
                display_path(repository_root, &path)
            ));
        } else if metadata.is_dir() {
            collect_rust_sources(repository_root, &path, source_files, violations);
        } else if metadata.is_file() && path.extension() == Some(OsStr::new("rs")) {
            source_files.push(path);
        }
    }
}

fn sorted_directory_entries(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_unstable();
    Ok(entries)
}

fn display_path(repository_root: &Path, path: &Path) -> String {
    path.strip_prefix(repository_root)
        .unwrap_or(path)
        .display()
        .to_string()
}
