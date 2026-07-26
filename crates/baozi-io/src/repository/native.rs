use std::cmp::Ordering;
use std::fmt;
use std::path::Path;

use cap_std::fs::{Dir, File};

use crate::{AssetOpenError, AuthorityDomain, NativeDirectoryCapability};

use super::{
    RepositoryCaptureError, RepositoryLimits, RepositoryObjectKind, RepositoryPath,
    RepositoryPathError, RepositoryVerifyError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ObjectIdentity {
    pub(super) device: u64,
    pub(super) object: u64,
}

/// One explicit file or tree root selected for repository capture.
#[derive(Debug, PartialEq, Eq)]
pub enum RepositoryRootSpec {
    File(RepositoryPath),
    Tree(RepositoryPath),
}

impl RepositoryRootSpec {
    pub fn path(&self) -> &RepositoryPath {
        match self {
            Self::File(path) | Self::Tree(path) => path,
        }
    }

    pub const fn object_kind(&self) -> RepositoryObjectKind {
        match self {
            Self::File(_) => RepositoryObjectKind::File,
            Self::Tree(_) => RepositoryObjectKind::Directory,
        }
    }

    pub fn try_clone(&self) -> Result<Self, RepositoryPathError> {
        match self {
            Self::File(path) => path.try_clone().map(Self::File),
            Self::Tree(path) => path.try_clone().map(Self::Tree),
        }
    }
}

/// Public resource accounting for one completed repository capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepositoryUsage {
    pub(super) roots: u64,
    pub(super) entries: u64,
    pub(super) files: u64,
    pub(super) directories: u64,
    pub(super) max_depth: u64,
    pub(super) max_path_bytes: u64,
    pub(super) max_file_bytes: u64,
    pub(super) total_bytes: u64,
    pub(super) work: u64,
}

impl RepositoryUsage {
    pub const fn roots(self) -> u64 {
        self.roots
    }

    pub const fn entries(self) -> u64 {
        self.entries
    }

    pub const fn files(self) -> u64 {
        self.files
    }

    pub const fn directories(self) -> u64 {
        self.directories
    }

    pub const fn max_depth(self) -> u64 {
        self.max_depth
    }

    pub const fn max_path_bytes(self) -> u64 {
        self.max_path_bytes
    }

    pub const fn max_file_bytes(self) -> u64 {
        self.max_file_bytes
    }

    pub const fn total_bytes(self) -> u64 {
        self.total_bytes
    }

    pub const fn work(self) -> u64 {
        self.work
    }
}

/// Captured bytes for one regular repository file.
pub struct RepositoryFile {
    pub(super) path: RepositoryPath,
    pub(super) bytes: Vec<u8>,
    pub(super) identity: ObjectIdentity,
    pub(super) _handle: File,
}

impl RepositoryFile {
    pub fn path(&self) -> &RepositoryPath {
        &self.path
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for RepositoryFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RepositoryFile")
            .field("path", &self.path)
            .field("byte_len", &self.bytes.len())
            .finish()
    }
}

pub(super) struct RepositoryDirectory {
    pub(super) path: RepositoryPath,
    pub(super) identity: ObjectIdentity,
    pub(super) children: Option<Vec<RepositoryDirectoryChild>>,
    pub(super) _handle: Dir,
}

/// One direct child observed in a captured directory snapshot.
pub(super) struct RepositoryDirectoryChild {
    pub(super) name: String,
    pub(super) kind: RepositoryObjectKind,
    pub(super) identity: ObjectIdentity,
}

/// A deterministic repository closure bound to one authority and finite policy.
pub struct RepositoryCapture {
    pub(super) authority: AuthorityDomain,
    pub(super) roots: Vec<RepositoryRootSpec>,
    pub(super) limits: RepositoryLimits,
    pub(super) usage: RepositoryUsage,
    pub(super) files: Vec<RepositoryFile>,
    pub(super) directories: Vec<RepositoryDirectory>,
}

impl RepositoryCapture {
    pub fn roots(&self) -> &[RepositoryRootSpec] {
        &self.roots
    }

    pub const fn limits(&self) -> RepositoryLimits {
        self.limits
    }

    pub const fn usage(&self) -> RepositoryUsage {
        self.usage
    }

    pub fn files(&self) -> &[RepositoryFile] {
        &self.files
    }

    /// Returns the captured directory paths in deterministic capture order.
    pub fn directory_paths(&self) -> impl ExactSizeIterator<Item = &RepositoryPath> {
        self.directories.iter().map(|directory| &directory.path)
    }
}

impl fmt::Debug for RepositoryCapture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RepositoryCapture")
            .field("roots", &self.roots)
            .field("limits", &self.limits)
            .field("usage", &self.usage)
            .field("files", &self.files)
            .field("directory_count", &self.directories.len())
            .finish_non_exhaustive()
    }
}

/// Retained native authority for strict repository capture and recapture.
pub struct RepositoryRoot {
    capability: NativeDirectoryCapability,
}

impl RepositoryRoot {
    pub fn open_ambient(root: impl AsRef<Path>) -> Result<Self, AssetOpenError> {
        NativeDirectoryCapability::open_ambient(root).map(Self::from_capability)
    }

    pub fn from_capability(capability: NativeDirectoryCapability) -> Self {
        Self { capability }
    }

    pub fn capture(
        &self,
        roots: Vec<RepositoryRootSpec>,
        limits: RepositoryLimits,
    ) -> Result<RepositoryCapture, RepositoryCaptureError> {
        super::walk::capture(&self.capability, roots, limits)
    }

    pub fn verify_unchanged(
        &self,
        baseline: &RepositoryCapture,
    ) -> Result<(), RepositoryVerifyError> {
        if self.capability.authority_domain() != &baseline.authority {
            return Err(RepositoryVerifyError::AuthorityMismatch);
        }

        let roots = try_clone_roots(&baseline.roots)?;
        let recaptured = match self.capture(roots, baseline.limits) {
            Ok(capture) => capture,
            Err(error) => {
                if error.kind() == super::RepositoryCaptureErrorKind::NotFound
                    && let Some(path) = error.path()
                {
                    return Err(RepositoryVerifyError::Removed {
                        path: clone_verify_path(path)?,
                    });
                }
                if let (
                    super::RepositoryCaptureErrorKind::ExpectedObject { expected, actual },
                    Some(path),
                ) = (error.kind(), error.path())
                {
                    return Err(RepositoryVerifyError::KindChanged {
                        path: clone_verify_path(path)?,
                        expected,
                        actual,
                    });
                }
                return Err(RepositoryVerifyError::Recapture(error));
            }
        };

        compare_captures(baseline, &recaptured)
    }
}

impl fmt::Debug for RepositoryRoot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RepositoryRoot")
            .field("authority", self.capability.authority_domain())
            .finish_non_exhaustive()
    }
}

fn try_clone_roots(
    roots: &[RepositoryRootSpec],
) -> Result<Vec<RepositoryRootSpec>, RepositoryVerifyError> {
    let mut cloned = Vec::new();
    cloned
        .try_reserve_exact(roots.len())
        .map_err(|_| RepositoryVerifyError::AllocationFailed)?;
    for root in roots {
        cloned.push(
            root.try_clone()
                .map_err(|_| RepositoryVerifyError::AllocationFailed)?,
        );
    }
    Ok(cloned)
}

#[derive(Clone, Copy)]
enum CapturedNode<'a> {
    File(&'a RepositoryFile),
    Directory(&'a RepositoryDirectory),
}

impl<'capture> CapturedNode<'capture> {
    fn path(self) -> &'capture RepositoryPath {
        match self {
            Self::File(file) => &file.path,
            Self::Directory(directory) => &directory.path,
        }
    }

    const fn kind(self) -> RepositoryObjectKind {
        match self {
            Self::File(_) => RepositoryObjectKind::File,
            Self::Directory(_) => RepositoryObjectKind::Directory,
        }
    }

    const fn identity(self) -> ObjectIdentity {
        match self {
            Self::File(file) => file.identity,
            Self::Directory(directory) => directory.identity,
        }
    }

    fn bytes(self) -> Option<&'capture [u8]> {
        match self {
            Self::File(file) => Some(&file.bytes),
            Self::Directory(_) => None,
        }
    }
}

struct CaptureCursor<'a> {
    capture: &'a RepositoryCapture,
    file: usize,
    directory: usize,
}

impl<'a> CaptureCursor<'a> {
    const fn new(capture: &'a RepositoryCapture) -> Self {
        Self {
            capture,
            file: 0,
            directory: 0,
        }
    }

    fn next(&mut self) -> Option<CapturedNode<'a>> {
        let file = self.capture.files.get(self.file);
        let directory = self.capture.directories.get(self.directory);
        match (file, directory) {
            (Some(file), Some(directory)) if file.path.as_str() <= directory.path.as_str() => {
                self.file += 1;
                Some(CapturedNode::File(file))
            }
            (Some(_), Some(directory)) => {
                self.directory += 1;
                Some(CapturedNode::Directory(directory))
            }
            (Some(file), None) => {
                self.file += 1;
                Some(CapturedNode::File(file))
            }
            (None, Some(directory)) => {
                self.directory += 1;
                Some(CapturedNode::Directory(directory))
            }
            (None, None) => None,
        }
    }
}

fn compare_captures(
    baseline: &RepositoryCapture,
    actual: &RepositoryCapture,
) -> Result<(), RepositoryVerifyError> {
    let mut expected_cursor = CaptureCursor::new(baseline);
    let mut actual_cursor = CaptureCursor::new(actual);
    let mut expected = expected_cursor.next();
    let mut observed = actual_cursor.next();

    loop {
        match (expected, observed) {
            (Some(left), Some(right)) => match left.path().cmp(right.path()) {
                Ordering::Less => {
                    return Err(RepositoryVerifyError::Removed {
                        path: clone_verify_path(left.path())?,
                    });
                }
                Ordering::Greater => {
                    return Err(RepositoryVerifyError::Added {
                        path: clone_verify_path(right.path())?,
                    });
                }
                Ordering::Equal => {
                    if left.kind() != right.kind() {
                        return Err(RepositoryVerifyError::KindChanged {
                            path: clone_verify_path(left.path())?,
                            expected: left.kind(),
                            actual: right.kind(),
                        });
                    }
                    if left.identity() != right.identity() {
                        return Err(RepositoryVerifyError::Replaced {
                            path: clone_verify_path(left.path())?,
                        });
                    }
                    if left.bytes() != right.bytes() {
                        return Err(RepositoryVerifyError::ContentChanged {
                            path: clone_verify_path(left.path())?,
                        });
                    }
                    expected = expected_cursor.next();
                    observed = actual_cursor.next();
                }
            },
            (Some(left), None) => {
                return Err(RepositoryVerifyError::Removed {
                    path: clone_verify_path(left.path())?,
                });
            }
            (None, Some(right)) => {
                return Err(RepositoryVerifyError::Added {
                    path: clone_verify_path(right.path())?,
                });
            }
            (None, None) => return Ok(()),
        }
    }
}

fn clone_verify_path(path: &RepositoryPath) -> Result<RepositoryPath, RepositoryVerifyError> {
    path.try_clone()
        .map_err(|_| RepositoryVerifyError::AllocationFailed)
}
