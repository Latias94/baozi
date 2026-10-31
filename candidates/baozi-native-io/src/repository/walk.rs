use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use cap_fs_ext::{
    DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsMaybeDirExt, OpenOptionsSyncExt,
};
use cap_std::fs::{Dir, File, FileType, Metadata, OpenOptions};

use crate::{NativeDirectoryCapability, fs::is_reparse_point};

use super::native::{ObjectIdentity, RepositoryDirectory, RepositoryDirectoryChild};
use super::path::{
    is_portable_alias, is_portable_path_prefix, paths_portably_equal, portable_component_cmp,
    portable_path_cmp, validate_component,
};
use super::{
    RepositoryCapture, RepositoryCaptureError, RepositoryCaptureErrorKind, RepositoryCaptureStage,
    RepositoryFile, RepositoryLimit, RepositoryLimits, RepositoryObjectKind, RepositoryPath,
    RepositoryRootSpec, RepositoryUsage,
};

const READ_BUFFER_BYTES: usize = 8 * 1024;

pub(super) fn capture(
    capability: &NativeDirectoryCapability,
    mut roots: Vec<RepositoryRootSpec>,
    limits: RepositoryLimits,
) -> Result<RepositoryCapture, RepositoryCaptureError> {
    let mut builder = CaptureBuilder::new(limits);
    validate_roots(&mut roots, &mut builder)?;
    ensure_identity_backend()?;

    let mut root_endpoints = Vec::new();
    root_endpoints.try_reserve_exact(roots.len()).map_err(|_| {
        RepositoryCaptureError::new(
            RepositoryCaptureStage::Finalize,
            RepositoryCaptureErrorKind::CapacityExceeded,
        )
    })?;
    for root in &roots {
        let endpoint = match root {
            RepositoryRootSpec::File(path) => capture_file_root(capability, path, &mut builder)?,
            RepositoryRootSpec::Tree(path) => capture_tree_root(capability, path, &mut builder)?,
        };
        root_endpoints.push(endpoint);
    }
    builder.finish(capability, roots, root_endpoints)
}

#[cfg(windows)]
fn ensure_identity_backend() -> Result<(), RepositoryCaptureError> {
    Err(RepositoryCaptureError::new(
        RepositoryCaptureStage::ValidateRoots,
        RepositoryCaptureErrorKind::UnsupportedIdentityBackend,
    ))
}

#[cfg(not(windows))]
const fn ensure_identity_backend() -> Result<(), RepositoryCaptureError> {
    Ok(())
}

struct CaptureBuilder {
    limits: RepositoryLimits,
    usage: RepositoryUsage,
    final_read_bytes: u64,
    files: Vec<RepositoryFile>,
    directories: Vec<RepositoryDirectory>,
    identities: Vec<IdentityRegistration>,
}

#[derive(Clone, Copy)]
enum IdentityLocation {
    File(usize),
    Directory(usize),
}

#[derive(Clone, Copy)]
struct IdentityRegistration {
    identity: ObjectIdentity,
    location: IdentityLocation,
}

impl CaptureBuilder {
    const fn new(limits: RepositoryLimits) -> Self {
        Self {
            limits,
            usage: RepositoryUsage {
                roots: 0,
                entries: 0,
                files: 0,
                directories: 0,
                max_depth: 0,
                max_path_bytes: 0,
                max_file_bytes: 0,
                total_bytes: 0,
                work: 0,
            },
            final_read_bytes: 0,
            files: Vec::new(),
            directories: Vec::new(),
            identities: Vec::new(),
        }
    }

    fn finish(
        mut self,
        capability: &NativeDirectoryCapability,
        roots: Vec<RepositoryRootSpec>,
        root_endpoints: Vec<EndpointObservation>,
    ) -> Result<RepositoryCapture, RepositoryCaptureError> {
        self.charge_sort_work(
            self.identities.len(),
            1,
            RepositoryCaptureStage::Finalize,
            None,
        )?;
        let identity_count = u64::try_from(self.identities.len()).map_err(|_| {
            RepositoryCaptureError::new(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::CapacityExceeded,
            )
        })?;
        self.charge_work(identity_count, RepositoryCaptureStage::Finalize, None)?;
        self.identities
            .sort_unstable_by_key(|registration| registration.identity);
        let duplicate = self
            .identities
            .windows(2)
            .find(|pair| pair[0].identity == pair[1].identity)
            .map(|pair| pair[1].location);
        if let Some(location) = duplicate {
            let Some(path) = self.identity_path(location) else {
                return Err(RepositoryCaptureError::new(
                    RepositoryCaptureStage::Finalize,
                    RepositoryCaptureErrorKind::Backend,
                ));
            };
            return Err(RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::IdentityAlias,
                path,
            ));
        }

        self.charge_sort_work(self.files.len(), 1, RepositoryCaptureStage::Finalize, None)?;
        self.files
            .sort_unstable_by(|left, right| left.path.as_str().cmp(right.path.as_str()));
        self.charge_sort_work(
            self.directories.len(),
            1,
            RepositoryCaptureStage::Finalize,
            None,
        )?;
        self.directories
            .sort_unstable_by(|left, right| left.path.as_str().cmp(right.path.as_str()));

        if roots.len() != root_endpoints.len() {
            return Err(RepositoryCaptureError::new(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::Backend,
            ));
        }
        let authority = capability.authority_domain().clone();
        for (root, expected) in roots.iter().zip(root_endpoints) {
            verify_root_endpoint(capability, root.path(), expected, &mut self)?;
        }
        let files = std::mem::take(&mut self.files);
        let directories = std::mem::take(&mut self.directories);
        verify_final_directory_closure(capability, &directories, &mut self)?;
        verify_final_file_closure(capability, &files, &mut self)?;

        Ok(RepositoryCapture {
            authority,
            roots,
            limits: self.limits,
            usage: self.usage,
            files,
            directories,
        })
    }

    fn identity_path(&self, location: IdentityLocation) -> Option<&RepositoryPath> {
        match location {
            IdentityLocation::File(index) => self.files.get(index).map(|file| &file.path),
            IdentityLocation::Directory(index) => {
                self.directories.get(index).map(|directory| &directory.path)
            }
        }
    }

    fn note_roots(&mut self, count: usize) -> Result<(), RepositoryCaptureError> {
        let attempted = u64::try_from(count).map_err(|_| {
            RepositoryCaptureError::new(
                RepositoryCaptureStage::ValidateRoots,
                RepositoryCaptureErrorKind::CapacityExceeded,
            )
        })?;
        if attempted > self.limits.max_roots() {
            return Err(self.limit_error(
                RepositoryCaptureStage::ValidateRoots,
                RepositoryLimit::Roots,
                attempted,
                self.limits.max_roots(),
                None,
            ));
        }
        self.usage.roots = attempted;
        Ok(())
    }

    fn note_node(
        &mut self,
        kind: EntryHint,
        stage: RepositoryCaptureStage,
        path: &RepositoryPath,
    ) -> Result<(), RepositoryCaptureError> {
        self.ensure_entry_capacity(stage, path)?;
        let entries = self.usage.entries.checked_add(1).ok_or_else(|| {
            RepositoryCaptureError::at(stage, RepositoryCaptureErrorKind::CapacityExceeded, path)
        })?;
        self.usage.entries = entries;

        let (limit, current, maximum) = match kind {
            EntryHint::File => (
                RepositoryLimit::Files,
                &mut self.usage.files,
                self.limits.max_files(),
            ),
            EntryHint::Directory => (
                RepositoryLimit::Directories,
                &mut self.usage.directories,
                self.limits.max_directories(),
            ),
            EntryHint::Unsupported(_) => return Ok(()),
        };
        let attempted = current.checked_add(1).ok_or_else(|| {
            RepositoryCaptureError::at(stage, RepositoryCaptureErrorKind::CapacityExceeded, path)
        })?;
        if attempted > maximum {
            return Err(RepositoryCaptureError::at(
                stage,
                RepositoryCaptureErrorKind::LimitExceeded {
                    limit,
                    attempted,
                    maximum,
                },
                path,
            ));
        }
        *current = attempted;
        Ok(())
    }

    fn ensure_entry_capacity(
        &self,
        stage: RepositoryCaptureStage,
        path: &RepositoryPath,
    ) -> Result<(), RepositoryCaptureError> {
        let attempted = self.usage.entries.checked_add(1).ok_or_else(|| {
            RepositoryCaptureError::at(stage, RepositoryCaptureErrorKind::CapacityExceeded, path)
        })?;
        if attempted > self.limits.max_entries() {
            return Err(RepositoryCaptureError::at(
                stage,
                RepositoryCaptureErrorKind::LimitExceeded {
                    limit: RepositoryLimit::Entries,
                    attempted,
                    maximum: self.limits.max_entries(),
                },
                path,
            ));
        }
        Ok(())
    }

    fn note_path_shape(
        &mut self,
        path_bytes: usize,
        depth: usize,
        stage: RepositoryCaptureStage,
        path: &RepositoryPath,
    ) -> Result<(), RepositoryCaptureError> {
        let path_bytes = u64::try_from(path_bytes).map_err(|_| {
            RepositoryCaptureError::at(stage, RepositoryCaptureErrorKind::CapacityExceeded, path)
        })?;
        if path_bytes > self.limits.max_path_bytes() {
            return Err(self.limit_error(
                stage,
                RepositoryLimit::PathBytes,
                path_bytes,
                self.limits.max_path_bytes(),
                Some(path),
            ));
        }
        let depth = u64::try_from(depth).map_err(|_| {
            RepositoryCaptureError::at(stage, RepositoryCaptureErrorKind::CapacityExceeded, path)
        })?;
        if depth > self.limits.max_depth() {
            return Err(self.limit_error(
                stage,
                RepositoryLimit::Depth,
                depth,
                self.limits.max_depth(),
                Some(path),
            ));
        }
        self.usage.max_path_bytes = self.usage.max_path_bytes.max(path_bytes);
        self.usage.max_depth = self.usage.max_depth.max(depth);
        Ok(())
    }

    fn check_child_path_shape(
        &mut self,
        parent: &RepositoryPath,
        component: &str,
        stage: RepositoryCaptureStage,
    ) -> Result<(), RepositoryCaptureError> {
        let path_bytes = parent.joined_len(component).ok_or_else(|| {
            RepositoryCaptureError::at(stage, RepositoryCaptureErrorKind::CapacityExceeded, parent)
        })?;
        let depth = parent.component_count().checked_add(1).ok_or_else(|| {
            RepositoryCaptureError::at(stage, RepositoryCaptureErrorKind::CapacityExceeded, parent)
        })?;
        self.note_path_shape(path_bytes, depth, stage, parent)
    }

    fn check_file_length_hint(
        &self,
        length: u64,
        path: &RepositoryPath,
    ) -> Result<(), RepositoryCaptureError> {
        if length > self.limits.max_file_bytes() {
            return Err(self.limit_error(
                RepositoryCaptureStage::InspectFile,
                RepositoryLimit::FileBytes,
                length,
                self.limits.max_file_bytes(),
                Some(path),
            ));
        }
        let attempted = self.usage.total_bytes.checked_add(length).ok_or_else(|| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::InspectFile,
                RepositoryCaptureErrorKind::CapacityExceeded,
                path,
            )
        })?;
        if attempted > self.limits.max_total_bytes() {
            return Err(self.limit_error(
                RepositoryCaptureStage::InspectFile,
                RepositoryLimit::TotalBytes,
                attempted,
                self.limits.max_total_bytes(),
                Some(path),
            ));
        }
        Ok(())
    }

    fn charge_final_read_bytes(
        &mut self,
        amount: u64,
        path: &RepositoryPath,
    ) -> Result<(), RepositoryCaptureError> {
        let attempted = self.final_read_bytes.checked_add(amount).ok_or_else(|| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::CapacityExceeded,
                path,
            )
        })?;
        if attempted > self.limits.max_total_bytes() {
            return Err(self.limit_error(
                RepositoryCaptureStage::Finalize,
                RepositoryLimit::TotalBytes,
                attempted,
                self.limits.max_total_bytes(),
                Some(path),
            ));
        }
        self.final_read_bytes = attempted;
        Ok(())
    }

    fn add_file(
        &mut self,
        path: RepositoryPath,
        bytes: Vec<u8>,
        identity: ObjectIdentity,
        handle: File,
    ) -> Result<(), RepositoryCaptureError> {
        let byte_count = u64::try_from(bytes.len()).map_err(|_| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::ReadFile,
                RepositoryCaptureErrorKind::CapacityExceeded,
                &path,
            )
        })?;
        let total = self
            .usage
            .total_bytes
            .checked_add(byte_count)
            .ok_or_else(|| {
                RepositoryCaptureError::at(
                    RepositoryCaptureStage::ReadFile,
                    RepositoryCaptureErrorKind::CapacityExceeded,
                    &path,
                )
            })?;
        if total > self.limits.max_total_bytes() {
            return Err(self.limit_error(
                RepositoryCaptureStage::ReadFile,
                RepositoryLimit::TotalBytes,
                total,
                self.limits.max_total_bytes(),
                Some(&path),
            ));
        }
        self.identities.try_reserve(1).map_err(|_| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::CapacityExceeded,
                &path,
            )
        })?;
        self.files.try_reserve(1).map_err(|_| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::CapacityExceeded,
                &path,
            )
        })?;
        self.usage.total_bytes = total;
        self.usage.max_file_bytes = self.usage.max_file_bytes.max(byte_count);
        let index = self.files.len();
        self.files.push(RepositoryFile {
            path,
            bytes,
            identity,
            _handle: handle,
        });
        self.identities.push(IdentityRegistration {
            identity,
            location: IdentityLocation::File(index),
        });
        Ok(())
    }

    fn add_directory(
        &mut self,
        path: RepositoryPath,
        identity: ObjectIdentity,
        handle: Dir,
    ) -> Result<usize, RepositoryCaptureError> {
        self.identities.try_reserve(1).map_err(|_| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::CapacityExceeded,
                &path,
            )
        })?;
        self.directories.try_reserve(1).map_err(|_| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::CapacityExceeded,
                &path,
            )
        })?;
        let index = self.directories.len();
        self.directories.push(RepositoryDirectory {
            path,
            identity,
            children: None,
            _handle: handle,
        });
        self.identities.push(IdentityRegistration {
            identity,
            location: IdentityLocation::Directory(index),
        });
        Ok(index)
    }

    fn set_directory_children(
        &mut self,
        index: usize,
        entries: Vec<DirectoryEntry>,
        path: &RepositoryPath,
    ) -> Result<(), RepositoryCaptureError> {
        let mut children = Vec::new();
        children.try_reserve_exact(entries.len()).map_err(|_| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::CapacityExceeded,
                path,
            )
        })?;
        for entry in entries {
            let DirectoryEntryState::Processed(endpoint) = entry.state else {
                return Err(RepositoryCaptureError::at(
                    RepositoryCaptureStage::Finalize,
                    RepositoryCaptureErrorKind::Backend,
                    path,
                ));
            };
            children.push(RepositoryDirectoryChild {
                name: entry.name,
                kind: endpoint.kind,
                identity: endpoint.identity,
            });
        }
        let Some(directory) = self.directories.get_mut(index) else {
            return Err(RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::Backend,
                path,
            ));
        };
        if directory.children.is_some() {
            return Err(RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::Backend,
                path,
            ));
        }
        directory.children = Some(children);
        Ok(())
    }

    fn charge_work(
        &mut self,
        amount: u64,
        stage: RepositoryCaptureStage,
        path: Option<&RepositoryPath>,
    ) -> Result<(), RepositoryCaptureError> {
        let attempted = self.usage.work.checked_add(amount).ok_or_else(|| {
            path.map_or_else(
                || RepositoryCaptureError::new(stage, RepositoryCaptureErrorKind::CapacityExceeded),
                |path| {
                    RepositoryCaptureError::at(
                        stage,
                        RepositoryCaptureErrorKind::CapacityExceeded,
                        path,
                    )
                },
            )
        })?;
        if attempted > self.limits.max_work() {
            return Err(self.limit_error(
                stage,
                RepositoryLimit::Work,
                attempted,
                self.limits.max_work(),
                path,
            ));
        }
        self.usage.work = attempted;
        Ok(())
    }

    fn charge_sort_work(
        &mut self,
        length: usize,
        passes: u64,
        stage: RepositoryCaptureStage,
        path: Option<&RepositoryPath>,
    ) -> Result<(), RepositoryCaptureError> {
        let length = u64::try_from(length).map_err(|_| {
            path.map_or_else(
                || RepositoryCaptureError::new(stage, RepositoryCaptureErrorKind::CapacityExceeded),
                |path| {
                    RepositoryCaptureError::at(
                        stage,
                        RepositoryCaptureErrorKind::CapacityExceeded,
                        path,
                    )
                },
            )
        })?;
        if length <= 1 {
            return Ok(());
        }
        let levels = u64::from(u64::BITS - (length - 1).leading_zeros());
        let amount = length
            .checked_mul(levels)
            .and_then(|amount| amount.checked_mul(passes))
            .ok_or_else(|| {
                path.map_or_else(
                    || {
                        RepositoryCaptureError::new(
                            stage,
                            RepositoryCaptureErrorKind::CapacityExceeded,
                        )
                    },
                    |path| {
                        RepositoryCaptureError::at(
                            stage,
                            RepositoryCaptureErrorKind::CapacityExceeded,
                            path,
                        )
                    },
                )
            })?;
        self.charge_work(amount, stage, path)
    }

    fn limit_error(
        &self,
        stage: RepositoryCaptureStage,
        limit: RepositoryLimit,
        attempted: u64,
        maximum: u64,
        path: Option<&RepositoryPath>,
    ) -> RepositoryCaptureError {
        let kind = RepositoryCaptureErrorKind::LimitExceeded {
            limit,
            attempted,
            maximum,
        };
        path.map_or_else(
            || RepositoryCaptureError::new(stage, kind),
            |path| RepositoryCaptureError::at(stage, kind, path),
        )
    }
}

fn validate_roots(
    roots: &mut Vec<RepositoryRootSpec>,
    builder: &mut CaptureBuilder,
) -> Result<(), RepositoryCaptureError> {
    if roots.is_empty() {
        return Err(RepositoryCaptureError::new(
            RepositoryCaptureStage::ValidateRoots,
            RepositoryCaptureErrorKind::NoRoots,
        ));
    }
    builder.note_roots(roots.len())?;
    builder.charge_sort_work(roots.len(), 2, RepositoryCaptureStage::ValidateRoots, None)?;

    roots.sort_unstable_by(|left, right| portable_path_cmp(left.path(), right.path()));
    for pair in roots.windows(2) {
        let left = pair[0].path();
        let right = pair[1].path();
        if paths_portably_equal(left, right) {
            let kind = if left == right {
                RepositoryCaptureErrorKind::DuplicateRoot
            } else {
                RepositoryCaptureErrorKind::PortableNameCollision
            };
            return Err(RepositoryCaptureError::at(
                RepositoryCaptureStage::ValidateRoots,
                kind,
                right,
            ));
        }
        if is_portable_path_prefix(left, right) || is_portable_path_prefix(right, left) {
            return Err(RepositoryCaptureError::at(
                RepositoryCaptureStage::ValidateRoots,
                RepositoryCaptureErrorKind::OverlappingRoots,
                right,
            ));
        }
    }

    roots.sort_unstable_by(|left, right| left.path().as_str().cmp(right.path().as_str()));
    for root in roots {
        builder.note_path_shape(
            root.path().as_str().len(),
            root.path().component_count(),
            RepositoryCaptureStage::ValidateRoots,
            root.path(),
        )?;
        let hint = match root {
            RepositoryRootSpec::File(_) => EntryHint::File,
            RepositoryRootSpec::Tree(_) => EntryHint::Directory,
        };
        builder.note_node(hint, RepositoryCaptureStage::ValidateRoots, root.path())?;
    }
    Ok(())
}

fn capture_file_root(
    capability: &NativeDirectoryCapability,
    path: &RepositoryPath,
    builder: &mut CaptureBuilder,
) -> Result<EndpointObservation, RepositoryCaptureError> {
    let (parent, component, diagnostic) = resolve_parent(capability, path, builder)?;
    let opened = open_file_component(&parent, component, &diagnostic, builder)?;
    let retained_path = path.try_clone().map_err(|error| {
        RepositoryCaptureError::at(
            RepositoryCaptureStage::Finalize,
            RepositoryCaptureErrorKind::InvalidPath(error.code()),
            path,
        )
    })?;
    let (bytes, identity, handle) = read_regular_file(opened, &retained_path, builder)?;
    let endpoint = EndpointObservation::new(RepositoryObjectKind::File, identity);
    builder.add_file(retained_path, bytes, identity, handle)?;
    Ok(endpoint)
}

fn capture_tree_root(
    capability: &NativeDirectoryCapability,
    path: &RepositoryPath,
    builder: &mut CaptureBuilder,
) -> Result<EndpointObservation, RepositoryCaptureError> {
    let (parent, component, diagnostic) = resolve_parent(capability, path, builder)?;
    let (directory, identity) = open_directory_component(&parent, component, &diagnostic, builder)?;
    let retained_path = path.try_clone().map_err(|error| {
        RepositoryCaptureError::at(
            RepositoryCaptureStage::Finalize,
            RepositoryCaptureErrorKind::InvalidPath(error.code()),
            path,
        )
    })?;
    capture_tree(directory, retained_path, identity, builder)?;
    Ok(EndpointObservation::new(
        RepositoryObjectKind::Directory,
        identity,
    ))
}

fn verify_root_endpoint(
    capability: &NativeDirectoryCapability,
    path: &RepositoryPath,
    expected: EndpointObservation,
    builder: &mut CaptureBuilder,
) -> Result<(), RepositoryCaptureError> {
    let (parent, component, diagnostic) = resolve_parent(capability, path, builder)?;
    let actual = observe_fresh_endpoint(
        &parent,
        component,
        &diagnostic,
        expected.kind,
        RepositoryCaptureStage::Finalize,
        builder,
    )?;
    compare_endpoint_observations(
        expected,
        actual,
        RepositoryCaptureStage::Finalize,
        &diagnostic,
    )
}

fn resolve_parent<'a>(
    capability: &NativeDirectoryCapability,
    path: &'a RepositoryPath,
    builder: &mut CaptureBuilder,
) -> Result<(Dir, &'a str, RepositoryPath), RepositoryCaptureError> {
    builder.charge_work(1, RepositoryCaptureStage::ResolveComponent, Some(path))?;
    let mut directory = capability.directory().try_clone().map_err(|error| {
        RepositoryCaptureError::from_io(RepositoryCaptureStage::ResolveComponent, error, Some(path))
    })?;
    let mut components = path.components().peekable();
    let mut traversed = String::new();
    traversed
        .try_reserve_exact(path.as_str().len())
        .map_err(|_| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::ResolveComponent,
                RepositoryCaptureErrorKind::CapacityExceeded,
                path,
            )
        })?;

    while let Some(component) = components.next() {
        if !traversed.is_empty() {
            traversed.push('/');
        }
        traversed.push_str(component);
        let diagnostic = RepositoryPath::from_valid_prefix(&traversed).map_err(|error| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::ResolveComponent,
                RepositoryCaptureErrorKind::InvalidPath(error.code()),
                path,
            )
        })?;
        ensure_exact_component(
            &directory,
            component,
            &diagnostic,
            RepositoryCaptureStage::ResolveComponent,
            builder,
        )?;
        if components.peek().is_none() {
            return Ok((directory, component, diagnostic));
        }
        let (next, _) = open_directory_component(&directory, component, &diagnostic, builder)?;
        ensure_exact_component(
            &directory,
            component,
            &diagnostic,
            RepositoryCaptureStage::ResolveComponent,
            builder,
        )?;
        directory = next;
    }

    Err(RepositoryCaptureError::at(
        RepositoryCaptureStage::ResolveComponent,
        RepositoryCaptureErrorKind::InvalidPath(super::RepositoryPathErrorCode::Empty),
        path,
    ))
}

fn ensure_exact_component(
    directory: &Dir,
    component: &str,
    path: &RepositoryPath,
    stage: RepositoryCaptureStage,
    builder: &mut CaptureBuilder,
) -> Result<(), RepositoryCaptureError> {
    builder.charge_work(1, stage, Some(path))?;
    let entries = directory
        .entries()
        .map_err(|error| RepositoryCaptureError::from_io(stage, error, Some(path)))?;
    let mut found_exact = false;
    let mut found_alias = false;
    for entry in entries {
        builder.charge_work(1, stage, Some(path))?;
        let entry =
            entry.map_err(|error| RepositoryCaptureError::from_io(stage, error, Some(path)))?;
        let name = entry.file_name();
        if name.as_os_str() == OsStr::new(component) {
            found_exact = true;
        } else if name
            .to_str()
            .is_some_and(|name| is_portable_alias(name, component))
        {
            found_alias = true;
        }
    }
    if found_alias && found_exact {
        return Err(RepositoryCaptureError::at(
            stage,
            RepositoryCaptureErrorKind::PortableNameCollision,
            path,
        ));
    }
    if found_alias {
        return Err(RepositoryCaptureError::at(
            stage,
            RepositoryCaptureErrorKind::ExactSpellingMismatch,
            path,
        ));
    }
    if found_exact {
        return Ok(());
    }
    Err(RepositoryCaptureError::at(
        stage,
        RepositoryCaptureErrorKind::NotFound,
        path,
    ))
}

struct OpenedFile {
    file: File,
    metadata: Metadata,
    identity: ObjectIdentity,
    links: u64,
}

fn open_file_component(
    parent: &Dir,
    component: &str,
    path: &RepositoryPath,
    builder: &mut CaptureBuilder,
) -> Result<OpenedFile, RepositoryCaptureError> {
    let (file, metadata) = open_filelike_component(parent, component, path, builder)?;
    reject_reparse(&metadata, RepositoryCaptureStage::InspectFile, path)?;
    let actual = classify_metadata(&metadata);
    if actual != RepositoryObjectKind::File {
        return Err(RepositoryCaptureError::at(
            RepositoryCaptureStage::InspectFile,
            RepositoryCaptureErrorKind::ExpectedObject {
                expected: RepositoryObjectKind::File,
                actual,
            },
            path,
        ));
    }
    let identity = inspect_identity(&metadata, RepositoryCaptureStage::InspectFile, path)?;
    if identity.links != 1 {
        return Err(RepositoryCaptureError::at(
            RepositoryCaptureStage::InspectFile,
            RepositoryCaptureErrorKind::HardLinkedFile {
                links: identity.links,
            },
            path,
        ));
    }
    builder.check_file_length_hint(metadata.len(), path)?;

    Ok(OpenedFile {
        file,
        metadata,
        identity: identity.identity,
        links: identity.links,
    })
}

fn open_filelike_component(
    parent: &Dir,
    component: &str,
    path: &RepositoryPath,
    builder: &mut CaptureBuilder,
) -> Result<(File, Metadata), RepositoryCaptureError> {
    builder.charge_work(1, RepositoryCaptureStage::OpenFile, Some(path))?;
    let mut options = OpenOptions::new();
    options.read(true);
    options.follow(FollowSymlinks::No);
    options.nonblock(true);
    options.maybe_dir(true);
    let file = parent
        .open_with(Path::new(component), &options)
        .map_err(|error| {
            RepositoryCaptureError::from_io(RepositoryCaptureStage::OpenFile, error, Some(path))
        })?;
    ensure_exact_component(
        parent,
        component,
        path,
        RepositoryCaptureStage::OpenFile,
        builder,
    )?;

    builder.charge_work(1, RepositoryCaptureStage::InspectFile, Some(path))?;
    let metadata = file.metadata().map_err(|error| {
        RepositoryCaptureError::from_io(RepositoryCaptureStage::InspectFile, error, Some(path))
    })?;
    Ok((file, metadata))
}

fn observe_fresh_endpoint(
    parent: &Dir,
    component: &str,
    path: &RepositoryPath,
    expected_kind: RepositoryObjectKind,
    stage: RepositoryCaptureStage,
    builder: &mut CaptureBuilder,
) -> Result<EndpointObservation, RepositoryCaptureError> {
    let metadata = if expected_kind == RepositoryObjectKind::Directory {
        builder.charge_work(1, stage, Some(path))?;
        match parent.open_dir_nofollow(Path::new(component)) {
            Ok(directory) => {
                ensure_exact_component(parent, component, path, stage, builder)?;
                builder.charge_work(1, stage, Some(path))?;
                directory
                    .dir_metadata()
                    .map_err(|error| RepositoryCaptureError::from_io(stage, error, Some(path)))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotADirectory => {
                let (_, metadata) = fresh_filelike_handle(parent, component, path, stage, builder)?;
                metadata
            }
            Err(error) => {
                return Err(RepositoryCaptureError::from_io(stage, error, Some(path)));
            }
        }
    } else {
        let (_, metadata) = fresh_filelike_handle(parent, component, path, stage, builder)?;
        metadata
    };

    endpoint_from_metadata(&metadata, stage, path)
}

fn fresh_filelike_handle(
    parent: &Dir,
    component: &str,
    path: &RepositoryPath,
    stage: RepositoryCaptureStage,
    builder: &mut CaptureBuilder,
) -> Result<(File, Metadata), RepositoryCaptureError> {
    builder.charge_work(1, stage, Some(path))?;
    let mut options = OpenOptions::new();
    options.read(true);
    options.follow(FollowSymlinks::No);
    options.nonblock(true);
    options.maybe_dir(true);
    let file = parent
        .open_with(Path::new(component), &options)
        .map_err(|error| RepositoryCaptureError::from_io(stage, error, Some(path)))?;
    ensure_exact_component(parent, component, path, stage, builder)?;
    builder.charge_work(1, stage, Some(path))?;
    let metadata = file
        .metadata()
        .map_err(|error| RepositoryCaptureError::from_io(stage, error, Some(path)))?;
    Ok((file, metadata))
}

fn endpoint_from_metadata(
    metadata: &Metadata,
    stage: RepositoryCaptureStage,
    path: &RepositoryPath,
) -> Result<EndpointObservation, RepositoryCaptureError> {
    reject_reparse(metadata, stage, path)?;
    let kind = classify_metadata(metadata);
    let identity = inspect_identity(metadata, stage, path)?;
    if kind == RepositoryObjectKind::File && identity.links != 1 {
        return Err(RepositoryCaptureError::at(
            stage,
            RepositoryCaptureErrorKind::HardLinkedFile {
                links: identity.links,
            },
            path,
        ));
    }
    Ok(EndpointObservation::new(kind, identity.identity))
}

fn open_fresh_directory_snapshot(
    parent: &Dir,
    component: &str,
    path: &RepositoryPath,
    expected: EndpointObservation,
    stage: RepositoryCaptureStage,
    builder: &mut CaptureBuilder,
) -> Result<Dir, RepositoryCaptureError> {
    builder.charge_work(1, stage, Some(path))?;
    let directory = match parent.open_dir_nofollow(Path::new(component)) {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotADirectory => {
            let (_, metadata) = fresh_filelike_handle(parent, component, path, stage, builder)?;
            let actual = endpoint_from_metadata(&metadata, stage, path)?;
            compare_endpoint_observations(expected, actual, stage, path)?;
            return Err(RepositoryCaptureError::at(
                stage,
                RepositoryCaptureErrorKind::Backend,
                path,
            ));
        }
        Err(error) => {
            return Err(RepositoryCaptureError::from_io(stage, error, Some(path)));
        }
    };
    ensure_exact_component(parent, component, path, stage, builder)?;
    builder.charge_work(1, stage, Some(path))?;
    let metadata = directory
        .dir_metadata()
        .map_err(|error| RepositoryCaptureError::from_io(stage, error, Some(path)))?;
    let actual = endpoint_from_metadata(&metadata, stage, path)?;
    compare_endpoint_observations(expected, actual, stage, path)?;
    Ok(directory)
}

pub(super) fn compare_endpoint_observations(
    expected: EndpointObservation,
    actual: EndpointObservation,
    stage: RepositoryCaptureStage,
    path: &RepositoryPath,
) -> Result<(), RepositoryCaptureError> {
    if expected != actual {
        return Err(RepositoryCaptureError::at(
            stage,
            RepositoryCaptureErrorKind::ObjectChangedDuringRead,
            path,
        ));
    }
    Ok(())
}

fn verify_final_file_closure(
    capability: &NativeDirectoryCapability,
    files: &[RepositoryFile],
    builder: &mut CaptureBuilder,
) -> Result<(), RepositoryCaptureError> {
    for expected in files {
        verify_final_file(capability, expected, builder)?;
    }
    if builder.final_read_bytes != builder.usage.total_bytes {
        return Err(RepositoryCaptureError::new(
            RepositoryCaptureStage::Finalize,
            RepositoryCaptureErrorKind::Backend,
        ));
    }
    Ok(())
}

fn verify_final_directory_closure(
    capability: &NativeDirectoryCapability,
    directories: &[RepositoryDirectory],
    builder: &mut CaptureBuilder,
) -> Result<(), RepositoryCaptureError> {
    for expected in directories {
        let (parent, component, diagnostic) = resolve_parent(capability, &expected.path, builder)?;
        let Some(expected_children) = expected.children.as_deref() else {
            return Err(RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::Backend,
                &diagnostic,
            ));
        };
        let directory = open_fresh_directory_snapshot(
            &parent,
            component,
            &diagnostic,
            EndpointObservation::new(RepositoryObjectKind::Directory, expected.identity),
            RepositoryCaptureStage::Finalize,
            builder,
        )?;
        let observed = enumerate_directory(
            &directory,
            &diagnostic,
            DirectoryEnumerationMode::VerifySnapshot {
                expected_entries: expected_children.len(),
            },
            RepositoryCaptureStage::Finalize,
            builder,
        )?;
        compare_directory_child_snapshot_names(
            expected_children,
            observed.iter().map(|entry| entry.name.as_str()),
            RepositoryCaptureStage::Finalize,
            &diagnostic,
        )?;
        for child in expected_children {
            let child_path = diagnostic.join_component(&child.name).map_err(|error| {
                RepositoryCaptureError::at(
                    RepositoryCaptureStage::Finalize,
                    RepositoryCaptureErrorKind::InvalidPath(error.code()),
                    &diagnostic,
                )
            })?;
            let actual = observe_fresh_endpoint(
                &directory,
                &child.name,
                &child_path,
                child.kind,
                RepositoryCaptureStage::Finalize,
                builder,
            )?;
            compare_endpoint_observations(
                EndpointObservation::new(child.kind, child.identity),
                actual,
                RepositoryCaptureStage::Finalize,
                &child_path,
            )?;
        }
    }
    Ok(())
}

fn verify_final_file(
    capability: &NativeDirectoryCapability,
    expected: &RepositoryFile,
    builder: &mut CaptureBuilder,
) -> Result<(), RepositoryCaptureError> {
    let path = &expected.path;
    let (parent, component, diagnostic) = resolve_parent(capability, path, builder)?;
    let (mut file, metadata) = fresh_filelike_handle(
        &parent,
        component,
        &diagnostic,
        RepositoryCaptureStage::Finalize,
        builder,
    )?;
    let expected_length = u64::try_from(expected.bytes.len()).map_err(|_| {
        RepositoryCaptureError::at(
            RepositoryCaptureStage::Finalize,
            RepositoryCaptureErrorKind::CapacityExceeded,
            &diagnostic,
        )
    })?;
    validate_read_metadata(
        &metadata,
        expected_length,
        expected_length,
        expected.identity,
        1,
        RepositoryCaptureStage::Finalize,
        &diagnostic,
    )?;
    let actual_identity =
        inspect_identity(&metadata, RepositoryCaptureStage::Finalize, &diagnostic)?;
    compare_endpoint_observations(
        EndpointObservation::new(RepositoryObjectKind::File, expected.identity),
        EndpointObservation::new(classify_metadata(&metadata), actual_identity.identity),
        RepositoryCaptureStage::Finalize,
        &diagnostic,
    )?;
    builder.charge_final_read_bytes(expected_length, &diagnostic)?;
    verify_final_file_bytes(&mut file, &expected.bytes, &diagnostic, builder)?;

    builder.charge_work(1, RepositoryCaptureStage::Finalize, Some(&diagnostic))?;
    let after = file.metadata().map_err(|error| {
        RepositoryCaptureError::from_io(RepositoryCaptureStage::Finalize, error, Some(&diagnostic))
    })?;
    validate_read_metadata(
        &after,
        expected_length,
        expected_length,
        expected.identity,
        1,
        RepositoryCaptureStage::Finalize,
        &diagnostic,
    )
}

fn verify_final_file_bytes(
    file: &mut File,
    expected: &[u8],
    path: &RepositoryPath,
    builder: &mut CaptureBuilder,
) -> Result<(), RepositoryCaptureError> {
    let mut offset = 0usize;
    let mut buffer = [0_u8; READ_BUFFER_BYTES];
    while offset < expected.len() {
        let remaining = expected.len().checked_sub(offset).ok_or_else(|| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::CapacityExceeded,
                path,
            )
        })?;
        let request = remaining.min(READ_BUFFER_BYTES);
        builder.charge_work(1, RepositoryCaptureStage::Finalize, Some(path))?;
        let count = file.read(&mut buffer[..request]).map_err(|error| {
            RepositoryCaptureError::from_io(RepositoryCaptureStage::Finalize, error, Some(path))
        })?;
        if count == 0 {
            return Err(RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::ObjectChangedDuringRead,
                path,
            ));
        }
        let end = offset.checked_add(count).ok_or_else(|| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::CapacityExceeded,
                path,
            )
        })?;
        let Some(expected_chunk) = expected.get(offset..end) else {
            return Err(RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::CapacityExceeded,
                path,
            ));
        };
        compare_final_file_bytes(expected_chunk, &buffer[..count], path)?;
        offset = end;
    }

    builder.charge_work(1, RepositoryCaptureStage::Finalize, Some(path))?;
    let count = file.read(&mut buffer[..1]).map_err(|error| {
        RepositoryCaptureError::from_io(RepositoryCaptureStage::Finalize, error, Some(path))
    })?;
    if count != 0 {
        return compare_final_file_bytes(&[], &buffer[..count], path);
    }
    Ok(())
}

pub(super) fn compare_final_file_bytes(
    expected: &[u8],
    actual: &[u8],
    path: &RepositoryPath,
) -> Result<(), RepositoryCaptureError> {
    if expected != actual {
        return Err(RepositoryCaptureError::at(
            RepositoryCaptureStage::Finalize,
            RepositoryCaptureErrorKind::ObjectChangedDuringRead,
            path,
        ));
    }
    Ok(())
}

fn open_directory_component(
    parent: &Dir,
    component: &str,
    path: &RepositoryPath,
    builder: &mut CaptureBuilder,
) -> Result<(Dir, ObjectIdentity), RepositoryCaptureError> {
    builder.charge_work(1, RepositoryCaptureStage::OpenDirectory, Some(path))?;
    let directory = match parent.open_dir_nofollow(Path::new(component)) {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotADirectory => {
            return inspect_directory_kind_mismatch(parent, component, path, builder);
        }
        Err(error) => {
            return Err(RepositoryCaptureError::from_io(
                RepositoryCaptureStage::OpenDirectory,
                error,
                Some(path),
            ));
        }
    };
    ensure_exact_component(
        parent,
        component,
        path,
        RepositoryCaptureStage::OpenDirectory,
        builder,
    )?;
    builder.charge_work(1, RepositoryCaptureStage::InspectDirectory, Some(path))?;
    let metadata = directory.dir_metadata().map_err(|error| {
        RepositoryCaptureError::from_io(RepositoryCaptureStage::InspectDirectory, error, Some(path))
    })?;
    reject_reparse(&metadata, RepositoryCaptureStage::InspectDirectory, path)?;
    let actual = classify_metadata(&metadata);
    if actual != RepositoryObjectKind::Directory {
        return Err(RepositoryCaptureError::at(
            RepositoryCaptureStage::InspectDirectory,
            RepositoryCaptureErrorKind::ExpectedObject {
                expected: RepositoryObjectKind::Directory,
                actual,
            },
            path,
        ));
    }
    let identity = inspect_identity(&metadata, RepositoryCaptureStage::InspectDirectory, path)?;
    Ok((directory, identity.identity))
}

fn inspect_directory_kind_mismatch(
    parent: &Dir,
    component: &str,
    path: &RepositoryPath,
    builder: &mut CaptureBuilder,
) -> Result<(Dir, ObjectIdentity), RepositoryCaptureError> {
    let (_file, metadata) = open_filelike_component(parent, component, path, builder)?;
    reject_reparse(&metadata, RepositoryCaptureStage::InspectFile, path)?;
    let actual = classify_metadata(&metadata);
    Err(RepositoryCaptureError::at(
        RepositoryCaptureStage::InspectFile,
        RepositoryCaptureErrorKind::ExpectedObject {
            expected: RepositoryObjectKind::Directory,
            actual,
        },
        path,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryHint {
    File,
    Directory,
    Unsupported(RepositoryObjectKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EndpointObservation {
    kind: RepositoryObjectKind,
    identity: ObjectIdentity,
}

impl EndpointObservation {
    pub(super) const fn new(kind: RepositoryObjectKind, identity: ObjectIdentity) -> Self {
        Self { kind, identity }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectoryEnumerationMode {
    CaptureChildren,
    VerifySnapshot { expected_entries: usize },
}

struct DirectoryEntry {
    name: String,
    state: DirectoryEntryState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectoryEntryState {
    Pending(EntryHint),
    Processed(EndpointObservation),
    Snapshot,
}

struct DirectoryFrame {
    directory: Dir,
    path: RepositoryPath,
    identity: ObjectIdentity,
    directory_index: usize,
    entries: Vec<DirectoryEntry>,
    next: usize,
}

struct ProcessedDirectoryEntry {
    endpoint: EndpointObservation,
    child_frame: Option<DirectoryFrame>,
}

fn capture_tree(
    directory: Dir,
    path: RepositoryPath,
    identity: ObjectIdentity,
    builder: &mut CaptureBuilder,
) -> Result<(), RepositoryCaptureError> {
    let retained_directory_path = path.try_clone().map_err(|error| {
        RepositoryCaptureError::at(
            RepositoryCaptureStage::Finalize,
            RepositoryCaptureErrorKind::InvalidPath(error.code()),
            &path,
        )
    })?;
    let retained_handle = retain_directory_handle(&directory, &path, builder)?;
    let directory_index =
        builder.add_directory(retained_directory_path, identity, retained_handle)?;
    let entries = enumerate_directory(
        &directory,
        &path,
        DirectoryEnumerationMode::CaptureChildren,
        RepositoryCaptureStage::EnumerateDirectory,
        builder,
    )?;
    let mut frames = Vec::new();
    frames.try_reserve(1).map_err(|_| {
        RepositoryCaptureError::at(
            RepositoryCaptureStage::EnumerateDirectory,
            RepositoryCaptureErrorKind::CapacityExceeded,
            &path,
        )
    })?;
    frames.push(DirectoryFrame {
        directory,
        path,
        identity,
        directory_index,
        entries,
        next: 0,
    });

    while !frames.is_empty() {
        let completed = frames
            .last()
            .is_some_and(|frame| frame.next == frame.entries.len());
        if completed {
            let Some(frame) = frames.pop() else {
                return Err(RepositoryCaptureError::new(
                    RepositoryCaptureStage::Finalize,
                    RepositoryCaptureErrorKind::Backend,
                ));
            };
            verify_completed_directory(frame, builder)?;
            continue;
        }

        let child_frame = {
            let Some(frame) = frames.last_mut() else {
                return Err(RepositoryCaptureError::new(
                    RepositoryCaptureStage::Finalize,
                    RepositoryCaptureErrorKind::Backend,
                ));
            };
            let entry_index = frame.next;
            let Some(entry) = frame.entries.get(entry_index) else {
                return Err(RepositoryCaptureError::at(
                    RepositoryCaptureStage::Finalize,
                    RepositoryCaptureErrorKind::Backend,
                    &frame.path,
                ));
            };
            let processed = process_directory_entry(&frame.directory, &frame.path, entry, builder)?;
            let Some(entry) = frame.entries.get_mut(entry_index) else {
                return Err(RepositoryCaptureError::at(
                    RepositoryCaptureStage::Finalize,
                    RepositoryCaptureErrorKind::Backend,
                    &frame.path,
                ));
            };
            entry.state = DirectoryEntryState::Processed(processed.endpoint);
            frame.next = entry_index.checked_add(1).ok_or_else(|| {
                RepositoryCaptureError::at(
                    RepositoryCaptureStage::Finalize,
                    RepositoryCaptureErrorKind::CapacityExceeded,
                    &frame.path,
                )
            })?;
            processed.child_frame
        };
        if let Some(frame) = child_frame {
            frames.try_reserve(1).map_err(|_| {
                RepositoryCaptureError::at(
                    RepositoryCaptureStage::EnumerateDirectory,
                    RepositoryCaptureErrorKind::CapacityExceeded,
                    &frame.path,
                )
            })?;
            frames.push(frame);
        }
    }
    Ok(())
}

fn process_directory_entry(
    parent: &Dir,
    parent_path: &RepositoryPath,
    entry: &DirectoryEntry,
    builder: &mut CaptureBuilder,
) -> Result<ProcessedDirectoryEntry, RepositoryCaptureError> {
    let path = parent_path.join_component(&entry.name).map_err(|error| {
        RepositoryCaptureError::at(
            RepositoryCaptureStage::ResolveComponent,
            RepositoryCaptureErrorKind::InvalidPath(error.code()),
            parent_path,
        )
    })?;
    match entry.state {
        DirectoryEntryState::Pending(EntryHint::File) => {
            let opened = open_file_component(parent, &entry.name, &path, builder)?;
            let (bytes, identity, handle) = read_regular_file(opened, &path, builder)?;
            builder.add_file(path, bytes, identity, handle)?;
            Ok(ProcessedDirectoryEntry {
                endpoint: EndpointObservation::new(RepositoryObjectKind::File, identity),
                child_frame: None,
            })
        }
        DirectoryEntryState::Pending(EntryHint::Directory) => {
            let (directory, identity) =
                open_directory_component(parent, &entry.name, &path, builder)?;
            let retained_path = path.try_clone().map_err(|error| {
                RepositoryCaptureError::at(
                    RepositoryCaptureStage::Finalize,
                    RepositoryCaptureErrorKind::InvalidPath(error.code()),
                    &path,
                )
            })?;
            let retained_handle = retain_directory_handle(&directory, &path, builder)?;
            let directory_index =
                builder.add_directory(retained_path, identity, retained_handle)?;
            let entries = enumerate_directory(
                &directory,
                &path,
                DirectoryEnumerationMode::CaptureChildren,
                RepositoryCaptureStage::EnumerateDirectory,
                builder,
            )?;
            Ok(ProcessedDirectoryEntry {
                endpoint: EndpointObservation::new(RepositoryObjectKind::Directory, identity),
                child_frame: Some(DirectoryFrame {
                    directory,
                    path,
                    identity,
                    directory_index,
                    entries,
                    next: 0,
                }),
            })
        }
        DirectoryEntryState::Pending(EntryHint::Unsupported(kind)) => {
            Err(RepositoryCaptureError::at(
                RepositoryCaptureStage::InspectFile,
                observe_unsupported_component(parent, &entry.name, &path, kind, builder)?,
                &path,
            ))
        }
        DirectoryEntryState::Processed(_) | DirectoryEntryState::Snapshot => {
            Err(RepositoryCaptureError::at(
                RepositoryCaptureStage::Finalize,
                RepositoryCaptureErrorKind::Backend,
                &path,
            ))
        }
    }
}

fn observe_unsupported_component(
    parent: &Dir,
    component: &str,
    path: &RepositoryPath,
    hinted: RepositoryObjectKind,
    builder: &mut CaptureBuilder,
) -> Result<RepositoryCaptureErrorKind, RepositoryCaptureError> {
    let (_file, metadata) = open_filelike_component(parent, component, path, builder)?;
    reject_reparse(&metadata, RepositoryCaptureStage::InspectFile, path)?;
    let actual = classify_metadata(&metadata);
    if actual != hinted {
        return Ok(RepositoryCaptureErrorKind::ExpectedObject {
            expected: hinted,
            actual,
        });
    }
    Ok(RepositoryCaptureErrorKind::UnsupportedObject(actual))
}

fn verify_completed_directory(
    frame: DirectoryFrame,
    builder: &mut CaptureBuilder,
) -> Result<(), RepositoryCaptureError> {
    let observed = enumerate_directory(
        &frame.directory,
        &frame.path,
        DirectoryEnumerationMode::VerifySnapshot {
            expected_entries: frame.entries.len(),
        },
        RepositoryCaptureStage::ReenumerateDirectory,
        builder,
    )?;
    if frame.entries.len() != observed.len()
        || frame
            .entries
            .iter()
            .zip(&observed)
            .any(|(before, after)| before.name != after.name)
    {
        return Err(RepositoryCaptureError::at(
            RepositoryCaptureStage::ReenumerateDirectory,
            RepositoryCaptureErrorKind::DirectoryEntriesChanged,
            &frame.path,
        ));
    }
    for entry in &frame.entries {
        let expected = match entry.state {
            DirectoryEntryState::Processed(endpoint) => endpoint,
            DirectoryEntryState::Pending(_) | DirectoryEntryState::Snapshot => {
                return Err(RepositoryCaptureError::at(
                    RepositoryCaptureStage::ReenumerateDirectory,
                    RepositoryCaptureErrorKind::Backend,
                    &frame.path,
                ));
            }
        };
        let child_path = frame.path.join_component(&entry.name).map_err(|error| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::ReenumerateDirectory,
                RepositoryCaptureErrorKind::InvalidPath(error.code()),
                &frame.path,
            )
        })?;
        let actual = observe_fresh_endpoint(
            &frame.directory,
            &entry.name,
            &child_path,
            expected.kind,
            RepositoryCaptureStage::ReenumerateDirectory,
            builder,
        )?;
        compare_endpoint_observations(
            expected,
            actual,
            RepositoryCaptureStage::ReenumerateDirectory,
            &child_path,
        )?;
    }

    builder.charge_work(
        1,
        RepositoryCaptureStage::InspectDirectory,
        Some(&frame.path),
    )?;
    let metadata = frame.directory.dir_metadata().map_err(|error| {
        RepositoryCaptureError::from_io(
            RepositoryCaptureStage::InspectDirectory,
            error,
            Some(&frame.path),
        )
    })?;
    reject_reparse(
        &metadata,
        RepositoryCaptureStage::InspectDirectory,
        &frame.path,
    )?;
    if classify_metadata(&metadata) != RepositoryObjectKind::Directory {
        return Err(RepositoryCaptureError::at(
            RepositoryCaptureStage::InspectDirectory,
            RepositoryCaptureErrorKind::ObjectChangedDuringRead,
            &frame.path,
        ));
    }
    let identity = inspect_identity(
        &metadata,
        RepositoryCaptureStage::InspectDirectory,
        &frame.path,
    )?;
    if identity.identity != frame.identity {
        return Err(RepositoryCaptureError::at(
            RepositoryCaptureStage::InspectDirectory,
            RepositoryCaptureErrorKind::ObjectChangedDuringRead,
            &frame.path,
        ));
    }
    let DirectoryFrame {
        path,
        directory_index,
        entries,
        ..
    } = frame;
    builder.set_directory_children(directory_index, entries, &path)
}

fn enumerate_directory(
    directory: &Dir,
    path: &RepositoryPath,
    mode: DirectoryEnumerationMode,
    stage: RepositoryCaptureStage,
    builder: &mut CaptureBuilder,
) -> Result<Vec<DirectoryEntry>, RepositoryCaptureError> {
    builder.charge_work(1, stage, Some(path))?;
    let entries = directory
        .entries()
        .map_err(|error| RepositoryCaptureError::from_io(stage, error, Some(path)))?;
    let mut retained = Vec::new();
    if let DirectoryEnumerationMode::VerifySnapshot { expected_entries } = mode {
        retained.try_reserve_exact(expected_entries).map_err(|_| {
            RepositoryCaptureError::at(stage, RepositoryCaptureErrorKind::CapacityExceeded, path)
        })?;
    }
    for entry in entries {
        builder.charge_work(1, stage, Some(path))?;
        let entry =
            entry.map_err(|error| RepositoryCaptureError::from_io(stage, error, Some(path)))?;
        match mode {
            DirectoryEnumerationMode::CaptureChildren => {
                builder.ensure_entry_capacity(stage, path)?;
            }
            DirectoryEnumerationMode::VerifySnapshot { expected_entries } => {
                ensure_snapshot_scratch_capacity(retained.len(), expected_entries, stage, path)?;
            }
        }
        let name = entry.file_name().into_string().map_err(|_| {
            RepositoryCaptureError::at(stage, RepositoryCaptureErrorKind::NonUtf8Name, path)
        })?;
        validate_component(&name, path.component_count()).map_err(|error| {
            RepositoryCaptureError::at(
                stage,
                RepositoryCaptureErrorKind::InvalidPath(error.code()),
                path,
            )
        })?;
        builder.check_child_path_shape(path, &name, stage)?;

        let state = if mode == DirectoryEnumerationMode::CaptureChildren {
            builder.charge_work(1, stage, Some(path))?;
            let file_type = entry
                .file_type()
                .map_err(|error| RepositoryCaptureError::from_io(stage, error, Some(path)))?;
            let hint = classify_entry_hint(&file_type);
            let child_path = path.join_component(&name).map_err(|error| {
                RepositoryCaptureError::at(
                    stage,
                    RepositoryCaptureErrorKind::InvalidPath(error.code()),
                    path,
                )
            })?;
            builder.note_node(hint, stage, &child_path)?;
            DirectoryEntryState::Pending(hint)
        } else {
            DirectoryEntryState::Snapshot
        };

        if mode == DirectoryEnumerationMode::CaptureChildren {
            retained.try_reserve(1).map_err(|_| {
                RepositoryCaptureError::at(
                    stage,
                    RepositoryCaptureErrorKind::CapacityExceeded,
                    path,
                )
            })?;
        }
        retained.push(DirectoryEntry { name, state });
    }

    builder.charge_sort_work(retained.len(), 2, stage, Some(path))?;
    retained.sort_unstable_by(|left, right| portable_component_cmp(&left.name, &right.name));
    if retained
        .windows(2)
        .any(|pair| is_portable_alias(&pair[0].name, &pair[1].name))
    {
        return Err(RepositoryCaptureError::at(
            stage,
            RepositoryCaptureErrorKind::PortableNameCollision,
            path,
        ));
    }
    retained.sort_unstable_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
    Ok(retained)
}

#[allow(dead_code)]
pub(super) fn ensure_reenumeration_scratch_capacity(
    retained_entries: usize,
    expected_entries: usize,
    path: &RepositoryPath,
) -> Result<(), RepositoryCaptureError> {
    ensure_snapshot_scratch_capacity(
        retained_entries,
        expected_entries,
        RepositoryCaptureStage::ReenumerateDirectory,
        path,
    )
}

fn ensure_snapshot_scratch_capacity(
    retained_entries: usize,
    expected_entries: usize,
    stage: RepositoryCaptureStage,
    path: &RepositoryPath,
) -> Result<(), RepositoryCaptureError> {
    if retained_entries >= expected_entries {
        return Err(RepositoryCaptureError::at(
            stage,
            RepositoryCaptureErrorKind::DirectoryEntriesChanged,
            path,
        ));
    }
    Ok(())
}

#[allow(dead_code)]
pub(super) fn compare_directory_snapshot_names<'name>(
    expected: &[String],
    observed: impl ExactSizeIterator<Item = &'name str>,
    stage: RepositoryCaptureStage,
    path: &RepositoryPath,
) -> Result<(), RepositoryCaptureError> {
    if expected.len() != observed.len() || !expected.iter().map(String::as_str).eq(observed) {
        return Err(RepositoryCaptureError::at(
            stage,
            RepositoryCaptureErrorKind::DirectoryEntriesChanged,
            path,
        ));
    }
    Ok(())
}

fn compare_directory_child_snapshot_names<'name>(
    expected: &[RepositoryDirectoryChild],
    observed: impl ExactSizeIterator<Item = &'name str>,
    stage: RepositoryCaptureStage,
    path: &RepositoryPath,
) -> Result<(), RepositoryCaptureError> {
    if expected.len() != observed.len()
        || !expected
            .iter()
            .map(|child| child.name.as_str())
            .eq(observed)
    {
        return Err(RepositoryCaptureError::at(
            stage,
            RepositoryCaptureErrorKind::DirectoryEntriesChanged,
            path,
        ));
    }
    Ok(())
}

fn retain_directory_handle(
    directory: &Dir,
    path: &RepositoryPath,
    builder: &mut CaptureBuilder,
) -> Result<Dir, RepositoryCaptureError> {
    builder.charge_work(1, RepositoryCaptureStage::Finalize, Some(path))?;
    directory.try_clone().map_err(|error| {
        RepositoryCaptureError::from_io(RepositoryCaptureStage::Finalize, error, Some(path))
    })
}

fn read_regular_file(
    mut opened: OpenedFile,
    path: &RepositoryPath,
    builder: &mut CaptureBuilder,
) -> Result<(Vec<u8>, ObjectIdentity, File), RepositoryCaptureError> {
    let expected_length = opened.metadata.len();
    let expected_length_usize = usize::try_from(expected_length).map_err(|_| {
        RepositoryCaptureError::at(
            RepositoryCaptureStage::ReadFile,
            RepositoryCaptureErrorKind::CapacityExceeded,
            path,
        )
    })?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(expected_length_usize)
        .map_err(|_| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::ReadFile,
                RepositoryCaptureErrorKind::CapacityExceeded,
                path,
            )
        })?;
    let mut buffer = [0_u8; READ_BUFFER_BYTES];

    while bytes.len() < expected_length_usize {
        let remaining = expected_length_usize - bytes.len();
        let request = remaining.min(READ_BUFFER_BYTES);
        builder.charge_work(1, RepositoryCaptureStage::ReadFile, Some(path))?;
        let count = opened.file.read(&mut buffer[..request]).map_err(|error| {
            RepositoryCaptureError::from_io(RepositoryCaptureStage::ReadFile, error, Some(path))
        })?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }

    let mut beyond_expected = [0_u8; 1];
    builder.charge_work(1, RepositoryCaptureStage::ReadFile, Some(path))?;
    if opened.file.read(&mut beyond_expected).map_err(|error| {
        RepositoryCaptureError::from_io(RepositoryCaptureStage::ReadFile, error, Some(path))
    })? != 0
    {
        return Err(RepositoryCaptureError::at(
            RepositoryCaptureStage::ReadFile,
            RepositoryCaptureErrorKind::ObjectChangedDuringRead,
            path,
        ));
    }

    builder.charge_work(1, RepositoryCaptureStage::InspectFile, Some(path))?;
    let after = opened.file.metadata().map_err(|error| {
        RepositoryCaptureError::from_io(RepositoryCaptureStage::InspectFile, error, Some(path))
    })?;
    let observed_length = u64::try_from(bytes.len()).map_err(|_| {
        RepositoryCaptureError::at(
            RepositoryCaptureStage::InspectFile,
            RepositoryCaptureErrorKind::CapacityExceeded,
            path,
        )
    })?;
    validate_read_metadata(
        &after,
        expected_length,
        observed_length,
        opened.identity,
        opened.links,
        RepositoryCaptureStage::InspectFile,
        path,
    )?;

    verify_second_read(&mut opened.file, &bytes, path, builder)?;
    builder.charge_work(1, RepositoryCaptureStage::InspectFile, Some(path))?;
    let final_metadata = opened.file.metadata().map_err(|error| {
        RepositoryCaptureError::from_io(RepositoryCaptureStage::InspectFile, error, Some(path))
    })?;
    validate_read_metadata(
        &final_metadata,
        expected_length,
        observed_length,
        opened.identity,
        opened.links,
        RepositoryCaptureStage::InspectFile,
        path,
    )?;
    Ok((bytes, opened.identity, opened.file))
}

fn verify_second_read(
    file: &mut File,
    expected: &[u8],
    path: &RepositoryPath,
    builder: &mut CaptureBuilder,
) -> Result<(), RepositoryCaptureError> {
    builder.charge_work(1, RepositoryCaptureStage::ReadFile, Some(path))?;
    let position = file.seek(SeekFrom::Start(0)).map_err(|error| {
        RepositoryCaptureError::from_io(RepositoryCaptureStage::ReadFile, error, Some(path))
    })?;
    if position != 0 {
        return Err(RepositoryCaptureError::at(
            RepositoryCaptureStage::ReadFile,
            RepositoryCaptureErrorKind::ObjectChangedDuringRead,
            path,
        ));
    }

    let mut offset = 0usize;
    let mut buffer = [0_u8; READ_BUFFER_BYTES];
    while offset < expected.len() {
        let remaining = expected.len().checked_sub(offset).ok_or_else(|| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::ReadFile,
                RepositoryCaptureErrorKind::CapacityExceeded,
                path,
            )
        })?;
        let request = remaining.min(READ_BUFFER_BYTES);
        builder.charge_work(1, RepositoryCaptureStage::ReadFile, Some(path))?;
        let count = file.read(&mut buffer[..request]).map_err(|error| {
            RepositoryCaptureError::from_io(RepositoryCaptureStage::ReadFile, error, Some(path))
        })?;
        if count == 0 {
            return Err(RepositoryCaptureError::at(
                RepositoryCaptureStage::ReadFile,
                RepositoryCaptureErrorKind::ObjectChangedDuringRead,
                path,
            ));
        }
        let end = offset.checked_add(count).ok_or_else(|| {
            RepositoryCaptureError::at(
                RepositoryCaptureStage::ReadFile,
                RepositoryCaptureErrorKind::CapacityExceeded,
                path,
            )
        })?;
        let Some(expected_chunk) = expected.get(offset..end) else {
            return Err(RepositoryCaptureError::at(
                RepositoryCaptureStage::ReadFile,
                RepositoryCaptureErrorKind::CapacityExceeded,
                path,
            ));
        };
        if buffer.get(..count) != Some(expected_chunk) {
            return Err(RepositoryCaptureError::at(
                RepositoryCaptureStage::ReadFile,
                RepositoryCaptureErrorKind::ObjectChangedDuringRead,
                path,
            ));
        }
        offset = end;
    }

    builder.charge_work(1, RepositoryCaptureStage::ReadFile, Some(path))?;
    if file.read(&mut buffer[..1]).map_err(|error| {
        RepositoryCaptureError::from_io(RepositoryCaptureStage::ReadFile, error, Some(path))
    })? != 0
    {
        return Err(RepositoryCaptureError::at(
            RepositoryCaptureStage::ReadFile,
            RepositoryCaptureErrorKind::ObjectChangedDuringRead,
            path,
        ));
    }
    Ok(())
}

fn validate_read_metadata(
    metadata: &Metadata,
    expected_length: u64,
    observed_length: u64,
    expected_identity: ObjectIdentity,
    expected_links: u64,
    stage: RepositoryCaptureStage,
    path: &RepositoryPath,
) -> Result<(), RepositoryCaptureError> {
    reject_reparse(metadata, stage, path)?;
    if classify_metadata(metadata) != RepositoryObjectKind::File {
        return Err(RepositoryCaptureError::at(
            stage,
            RepositoryCaptureErrorKind::ObjectChangedDuringRead,
            path,
        ));
    }
    let identity = inspect_identity(metadata, stage, path)?;
    if identity.links != 1 {
        return Err(RepositoryCaptureError::at(
            stage,
            RepositoryCaptureErrorKind::HardLinkedFile {
                links: identity.links,
            },
            path,
        ));
    }
    if identity.identity != expected_identity
        || identity.links != expected_links
        || metadata.len() != expected_length
        || observed_length != expected_length
    {
        return Err(RepositoryCaptureError::at(
            stage,
            RepositoryCaptureErrorKind::ObjectChangedDuringRead,
            path,
        ));
    }
    Ok(())
}

struct IdentityObservation {
    identity: ObjectIdentity,
    links: u64,
}

fn inspect_identity(
    metadata: &Metadata,
    stage: RepositoryCaptureStage,
    path: &RepositoryPath,
) -> Result<IdentityObservation, RepositoryCaptureError> {
    catch_unwind(AssertUnwindSafe(|| IdentityObservation {
        identity: ObjectIdentity {
            device: cap_fs_ext::MetadataExt::dev(metadata),
            object: cap_fs_ext::MetadataExt::ino(metadata),
        },
        links: cap_fs_ext::MetadataExt::nlink(metadata),
    }))
    .map_err(|_| {
        RepositoryCaptureError::at(stage, RepositoryCaptureErrorKind::IdentityUnavailable, path)
    })
}

fn reject_reparse(
    metadata: &Metadata,
    stage: RepositoryCaptureStage,
    path: &RepositoryPath,
) -> Result<(), RepositoryCaptureError> {
    if is_reparse_point(metadata) {
        return Err(RepositoryCaptureError::at(
            stage,
            RepositoryCaptureErrorKind::UnsupportedObject(
                RepositoryObjectKind::SymlinkOrReparsePoint,
            ),
            path,
        ));
    }
    Ok(())
}

fn classify_metadata(metadata: &Metadata) -> RepositoryObjectKind {
    if metadata.is_file() {
        RepositoryObjectKind::File
    } else if metadata.is_dir() {
        RepositoryObjectKind::Directory
    } else if metadata.is_symlink() {
        RepositoryObjectKind::SymlinkOrReparsePoint
    } else {
        classify_platform_file_type(&metadata.file_type())
    }
}

fn classify_entry_hint(file_type: &FileType) -> EntryHint {
    if file_type.is_file() {
        EntryHint::File
    } else if file_type.is_dir() {
        EntryHint::Directory
    } else if file_type.is_symlink() {
        EntryHint::Unsupported(RepositoryObjectKind::SymlinkOrReparsePoint)
    } else {
        EntryHint::Unsupported(classify_platform_file_type(file_type))
    }
}

#[cfg(unix)]
fn classify_platform_file_type(file_type: &FileType) -> RepositoryObjectKind {
    use cap_std::fs::FileTypeExt;

    if file_type.is_fifo() {
        RepositoryObjectKind::Fifo
    } else if file_type.is_socket() {
        RepositoryObjectKind::Socket
    } else if file_type.is_char_device() {
        RepositoryObjectKind::CharacterDevice
    } else if file_type.is_block_device() {
        RepositoryObjectKind::BlockDevice
    } else {
        RepositoryObjectKind::Other
    }
}

#[cfg(windows)]
const fn classify_platform_file_type(_file_type: &FileType) -> RepositoryObjectKind {
    RepositoryObjectKind::Other
}
