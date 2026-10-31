use crate::{
    AssetIo, AssetLength, AssetObjectKind, AssetOpenError, AssetOpenErrorKind, AssetOpenStage,
    AssetPath, AuthorityDomain, OpenedAsset,
};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
#[cfg(unix)]
use cap_std::fs::{FileTypeExt, OpenOptionsExt};
use std::fmt;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct NativeDirectoryCapability {
    domain: AuthorityDomain,
    directory: Arc<Dir>,
}

impl NativeDirectoryCapability {
    /// Opens a trusted host directory before any hostile asset reference is processed.
    pub fn open_ambient(root: impl AsRef<Path>) -> Result<Self, AssetOpenError> {
        let directory = Dir::open_ambient_dir(root, ambient_authority())
            .map_err(|error| AssetOpenError::from_io(AssetOpenStage::RootCapability, error))?;
        let metadata = directory
            .dir_metadata()
            .map_err(|error| AssetOpenError::from_io(AssetOpenStage::RootCapability, error))?;
        if !metadata.is_dir() {
            return Err(AssetOpenError::new(
                AssetOpenStage::RootCapability,
                AssetOpenErrorKind::RootNotDirectory,
            ));
        }

        Ok(Self {
            domain: AuthorityDomain::new(),
            directory: Arc::new(directory),
        })
    }

    pub fn authority_domain(&self) -> &AuthorityDomain {
        &self.domain
    }

    #[cfg(feature = "repository-fs-candidate")]
    pub(crate) fn directory(&self) -> &Dir {
        &self.directory
    }
}

impl fmt::Debug for NativeDirectoryCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeDirectoryCapability")
            .field("domain", &self.domain)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct FileSystemAssetIo {
    capability: NativeDirectoryCapability,
}

impl FileSystemAssetIo {
    pub fn from_capability(capability: NativeDirectoryCapability) -> Self {
        Self { capability }
    }
}

impl AssetIo for FileSystemAssetIo {
    fn authority_domain(&self) -> AuthorityDomain {
        self.capability.domain.clone()
    }

    fn open(&self, path: &AssetPath) -> Result<OpenedAsset, AssetOpenError> {
        let retained_path = path.try_clone().map_err(|_| {
            AssetOpenError::new(AssetOpenStage::Open, AssetOpenErrorKind::CapacityExceeded)
        })?;
        let mut options = OpenOptions::new();
        options.read(true);
        configure_nonblocking(&mut options);

        let file = self
            .capability
            .directory
            .open_with(Path::new(path.as_str()), &options)
            .map_err(|error| AssetOpenError::from_io(AssetOpenStage::Open, error))?;
        let metadata = file
            .metadata()
            .map_err(|error| AssetOpenError::from_io(AssetOpenStage::InspectObject, error))?;
        // Unknown Windows reparse tags can still report a regular file on this handle.
        if is_reparse_point(&metadata) {
            return Err(AssetOpenError::new(
                AssetOpenStage::InspectObject,
                AssetOpenErrorKind::NonRegularFile(AssetObjectKind::SymlinkOrReparsePoint),
            ));
        }
        let file_type = metadata.file_type();
        if !file_type.is_file() {
            return Err(AssetOpenError::new(
                AssetOpenStage::InspectObject,
                AssetOpenErrorKind::NonRegularFile(classify_object(file_type)),
            ));
        }

        Ok(OpenedAsset::new(
            self.capability.domain.clone(),
            retained_path,
            file,
            Some(AssetLength::Hint(metadata.len())),
        ))
    }
}

#[cfg(unix)]
fn configure_nonblocking(options: &mut OpenOptions) {
    options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
}

#[cfg(windows)]
fn configure_nonblocking(_options: &mut OpenOptions) {}

#[cfg(windows)]
pub(crate) fn is_reparse_point(metadata: &cap_std::fs::Metadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    cap_std::fs::MetadataExt::file_attributes(metadata) & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
pub(crate) fn is_reparse_point(metadata: &cap_std::fs::Metadata) -> bool {
    metadata.is_symlink()
}

fn classify_object(file_type: cap_std::fs::FileType) -> AssetObjectKind {
    if file_type.is_dir() {
        return AssetObjectKind::Directory;
    }
    classify_platform_object(file_type)
}

#[cfg(unix)]
fn classify_platform_object(file_type: cap_std::fs::FileType) -> AssetObjectKind {
    if file_type.is_fifo() {
        AssetObjectKind::Fifo
    } else if file_type.is_socket() {
        AssetObjectKind::Socket
    } else if file_type.is_char_device() {
        AssetObjectKind::CharacterDevice
    } else if file_type.is_block_device() {
        AssetObjectKind::BlockDevice
    } else {
        AssetObjectKind::Other
    }
}

#[cfg(windows)]
fn classify_platform_object(_file_type: cap_std::fs::FileType) -> AssetObjectKind {
    AssetObjectKind::Other
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("baozi-io-{name}-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).expect("temp root should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn read_all(mut asset: OpenedAsset) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 16];
        loop {
            let read = asset.read_chunk(&mut chunk).expect("native read");
            if read == 0 {
                return bytes;
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
    }

    fn assert_bounded_replacement_race(
        io: &FileSystemAssetIo,
        request: &AssetPath,
        active: &Path,
        alternate: &Path,
        expected: [Option<&[u8]>; 2],
    ) {
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        const ROUNDS: usize = 64;
        const TIMEOUT: Duration = Duration::from_secs(5);
        let parked = active.with_file_name("race-parked");
        assert!(!parked.exists());
        let check_current = |state: usize| match expected[state] {
            Some(bytes) => assert_eq!(
                read_all(io.open(request).expect("stable authorized path must open")),
                bytes
            ),
            None => assert!(
                io.open(request).is_err(),
                "stable escaping path must fail the negative control"
            ),
        };
        check_current(0);
        // Timeouts detect stalled coordination; joining cannot interrupt a hung OS open/read.
        thread::scope(|threads| {
            let (start, starts) = mpsc::sync_channel::<()>(0);
            let (read, reads) = mpsc::channel::<()>();
            let (result, results) = mpsc::channel();
            let reader = threads.spawn(move || {
                for _ in 0..ROUNDS {
                    if starts.recv_timeout(TIMEOUT).is_err() {
                        return;
                    }
                    let opened = io.open(request);
                    // Keep the returned handle until the cooperating replacement has completed.
                    if reads.recv_timeout(TIMEOUT).is_err() {
                        return;
                    }
                    let observed = opened.map(read_all);
                    // read_all has dropped the handle before the replacer restores the parked path.
                    if result.send(observed).is_err() {
                        return;
                    }
                }
            });
            let mut state = 0;
            for round in 0..ROUNDS {
                start.send(()).expect("reader must accept the next round");
                let moved = match fs::rename(active, &parked) {
                    Ok(()) => true,
                    #[cfg(windows)]
                    Err(error) if matches!(error.raw_os_error(), Some(5 | 32)) => false,
                    Err(error) => panic!("round {round}: unexpected replacement failure: {error}"),
                };
                if moved {
                    fs::rename(alternate, active).expect("install the prewritten alternate entry");
                }
                read.send(())
                    .expect("reader must accept replacement completion");
                let observed = results
                    .recv_timeout(TIMEOUT)
                    .expect("reader did not report within the coordination timeout");
                if let Ok(bytes) = observed {
                    assert!(
                        expected.iter().flatten().any(|allowed| bytes == *allowed),
                        "round {round}: read outside or mixed bytes from the returned handle: {bytes:?}"
                    );
                }
                if !moved {
                    // Windows may deny the directory itself (32) or an open descendant (5).
                    // Complete the same exchange once that handle has been dropped; never skip it.
                    fs::rename(active, &parked)
                        .expect("replacement must succeed after the reader releases its handle");
                    fs::rename(alternate, active).expect("install the alternate after release");
                }
                fs::rename(&parked, alternate).expect("restore the parked entry as the alternate");
                state ^= 1;
                check_current(state);
            }
            reader
                .join()
                .expect("cooperating reader must finish all rounds");
        });
        check_current(0);
        assert!(!parked.exists());
    }

    #[cfg(windows)]
    fn replace_retained_root(root_path: &Path, moved_path: &Path) {
        let error = fs::rename(root_path, moved_path)
            .expect_err("Windows capability root must deny rename while retained");
        assert!(
            error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(32),
            "Windows may only reject root replacement with permission denied or ERROR_SHARING_VIOLATION (32): {error}"
        );
        assert_eq!(
            fs::read(root_path.join("identity.bin"))
                .expect("read unchanged fixture through host path"),
            b"authorized"
        );
    }

    #[cfg(not(windows))]
    fn replace_retained_root(root_path: &Path, moved_path: &Path) {
        fs::rename(root_path, moved_path)
            .expect("candidate runner must replace the retained root fixture");
        fs::create_dir(root_path).expect("create replacement root");
        fs::write(root_path.join("identity.bin"), b"replacement")
            .expect("write replacement fixture");
        assert_eq!(
            fs::read(root_path.join("identity.bin"))
                .expect("read replacement fixture through host path"),
            b"replacement"
        );
    }

    fn replace_opened_file_fixture(source: &Path, moved: &Path) {
        match fs::rename(source, moved) {
            Ok(()) => {
                fs::write(source, b"replacement").expect("write replacement fixture");
                assert_eq!(
                    fs::read(source).expect("read replacement fixture through host path"),
                    b"replacement"
                );
            }
            #[cfg(windows)]
            Err(error) => {
                assert!(
                    error.kind() == std::io::ErrorKind::PermissionDenied
                        || error.raw_os_error() == Some(32),
                    "Windows may only reject file replacement with permission denied or ERROR_SHARING_VIOLATION (32): {error}"
                );
                assert_eq!(
                    fs::read(source).expect("read unchanged fixture through host path"),
                    b"authorized"
                );
            }
            #[cfg(not(windows))]
            Err(error) => panic!("candidate runner must replace the opened file fixture: {error}"),
        }
    }

    #[test]
    fn opens_and_inspects_the_same_regular_file_handle() {
        let root = TempRoot::new("inside-scope");
        fs::write(root.path().join("mesh.stl"), b"solid test").expect("write fixture");
        let capability =
            NativeDirectoryCapability::open_ambient(root.path()).expect("open capability");
        let io = FileSystemAssetIo::from_capability(capability.clone());

        let opened = io
            .open(&AssetPath::new("mesh.stl").expect("valid path"))
            .expect("open asset");

        assert_eq!(opened.length(), Some(AssetLength::Hint(10)));
        assert_eq!(opened.authority_domain(), capability.authority_domain());
        assert_eq!(read_all(opened), b"solid test");
    }

    #[test]
    fn rejects_non_directory_roots_and_non_regular_assets() {
        let root = TempRoot::new("object-types");
        fs::write(root.path().join("not-a-root"), b"file").expect("write fixture");
        let root_error = NativeDirectoryCapability::open_ambient(root.path().join("not-a-root"))
            .expect_err("file root must fail");
        assert_eq!(root_error.stage(), AssetOpenStage::RootCapability);
        assert_eq!(root_error.kind(), AssetOpenErrorKind::RootNotDirectory);

        fs::create_dir(root.path().join("directory.asset")).expect("create directory fixture");
        let capability =
            NativeDirectoryCapability::open_ambient(root.path()).expect("open capability");
        let io = FileSystemAssetIo::from_capability(capability);
        let error = io
            .open(&AssetPath::new("directory.asset").expect("valid path"))
            .expect_err("directory must fail");

        assert!(matches!(
            error.stage(),
            AssetOpenStage::Open | AssetOpenStage::InspectObject
        ));
        if error.stage() == AssetOpenStage::InspectObject {
            assert_eq!(
                error.kind(),
                AssetOpenErrorKind::NonRegularFile(AssetObjectKind::Directory)
            );
        }
    }

    #[test]
    fn rejects_symlink_escape() {
        let root = TempRoot::new("symlink-root");
        let outside = TempRoot::new("symlink-outside");
        fs::write(outside.path().join("secret.stl"), b"secret").expect("write fixture");
        let link = root.path().join("link.stl");
        create_file_symlink(&outside.path().join("secret.stl"), &link)
            .expect("candidate runner must create a file symlink fixture");

        let capability =
            NativeDirectoryCapability::open_ambient(root.path()).expect("open capability");
        let io = FileSystemAssetIo::from_capability(capability);

        let result = io.open(&AssetPath::new("link.stl").expect("valid path"));
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape_when_the_link_target_ends_in_a_slash() {
        let root = TempRoot::new("symlink-trailing-target-root");
        let outside = TempRoot::new("symlink-trailing-target-outside");
        fs::write(outside.path().join("secret.stl"), b"secret").expect("write fixture");
        let link = root.path().join("escape");
        let target = format!("{}/", outside.path().display());
        create_file_symlink(Path::new(&target), &link)
            .expect("candidate runner must create a trailing-slash directory symlink");

        let capability =
            NativeDirectoryCapability::open_ambient(root.path()).expect("open capability");
        let io = FileSystemAssetIo::from_capability(capability);
        let path = AssetPath::new("escape/secret.stl").expect("ordinary request path");

        assert!(
            io.open(&path).is_err(),
            "symlink target must stay in the capability"
        );
    }

    #[test]
    fn follows_in_root_symlink_without_reopening_by_host_path() {
        let root = TempRoot::new("in-root-symlink");
        fs::write(root.path().join("target.bin"), b"authorized").expect("write fixture");
        let link = root.path().join("link.bin");
        create_file_symlink(Path::new("target.bin"), &link)
            .expect("candidate runner must create an in-root file symlink fixture");

        let capability =
            NativeDirectoryCapability::open_ambient(root.path()).expect("open capability");
        let io = FileSystemAssetIo::from_capability(capability);
        let opened = io
            .open(&AssetPath::new("link.bin").expect("valid path"))
            .expect("in-root symlink remains within the capability");

        assert_eq!(read_all(opened), b"authorized");
    }

    #[test]
    fn retained_root_capability_cannot_be_redirected_by_textual_replacement() {
        let parent = TempRoot::new("root-replacement");
        let root_path = parent.path().join("root");
        let moved_path = parent.path().join("moved-root");
        fs::create_dir(&root_path).expect("create root");
        fs::write(root_path.join("identity.bin"), b"authorized").expect("write fixture");

        let capability =
            NativeDirectoryCapability::open_ambient(&root_path).expect("open capability");
        let io = FileSystemAssetIo::from_capability(capability);

        replace_retained_root(&root_path, &moved_path);

        let opened = io
            .open(&AssetPath::new("identity.bin").expect("valid path"))
            .expect("retained root opens original object");
        assert_eq!(read_all(opened), b"authorized");
    }

    #[test]
    fn opened_file_remains_bound_after_final_path_replacement() {
        let root = TempRoot::new("file-replacement");
        let source = root.path().join("identity.bin");
        let moved = root.path().join("original.bin");
        fs::write(&source, b"authorized").expect("write fixture");
        let capability =
            NativeDirectoryCapability::open_ambient(root.path()).expect("open capability");
        let io = FileSystemAssetIo::from_capability(capability);
        let opened = io
            .open(&AssetPath::new("identity.bin").expect("valid path"))
            .expect("open original handle");

        replace_opened_file_fixture(&source, &moved);

        assert_eq!(read_all(opened), b"authorized");
    }

    #[test]
    fn final_file_replacement_race_keeps_immutable_handle_bytes() {
        let root = TempRoot::new("final-file-race");
        let active = root.path().join("entry.bin");
        let alternate = root.path().join("alternate.bin");
        let first = [b'A'; 64];
        let second = [b'B'; 64];
        fs::write(&active, first).expect("write the first immutable file");
        fs::write(&alternate, second).expect("write the second immutable file");
        let io = FileSystemAssetIo::from_capability(
            NativeDirectoryCapability::open_ambient(root.path()).expect("open race capability"),
        );
        assert_eq!(
            read_all(io.open(&AssetPath::new("alternate.bin").unwrap()).unwrap()),
            second
        );
        assert!(io.open(&AssetPath::new("missing.bin").unwrap()).is_err());

        assert_bounded_replacement_race(
            &io,
            &AssetPath::new("entry.bin").unwrap(),
            &active,
            &alternate,
            [Some(first.as_slice()), Some(second.as_slice())],
        );
        assert_eq!(fs::read(active).unwrap(), first);
        assert_eq!(fs::read(alternate).unwrap(), second);
    }

    #[test]
    fn directory_replacement_race_never_reads_outside_bytes() {
        let root = TempRoot::new("directory-race");
        let outside = TempRoot::new("directory-race-outside");
        let active = root.path().join("entry");
        let alternate = root.path().join("alternate");
        fs::create_dir(&active).expect("create the real in-root directory");
        fs::write(active.join("payload.bin"), b"authorized immutable payload")
            .expect("write the authorized immutable file");
        fs::write(
            outside.path().join("payload.bin"),
            b"outside immutable payload",
        )
        .expect("write the outside immutable file");
        #[cfg(unix)]
        create_file_symlink(outside.path(), &alternate)
            .expect("create the escaping directory link");
        #[cfg(windows)]
        create_junction(outside.path(), &alternate)
            .expect("create the escaping directory junction");
        assert_eq!(
            fs::read(alternate.join("payload.bin")).unwrap(),
            b"outside immutable payload"
        );
        let io = FileSystemAssetIo::from_capability(
            NativeDirectoryCapability::open_ambient(root.path()).expect("open race capability"),
        );
        assert!(
            io.open(&AssetPath::new("alternate/payload.bin").unwrap())
                .is_err()
        );
        #[cfg(windows)]
        {
            let held_child = io
                .open(&AssetPath::new("entry/payload.bin").unwrap())
                .expect("open the authorized child before the rename control");
            let parked = root.path().join("held-child-parked");
            let error = fs::rename(&active, &parked)
                .expect_err("Windows must deny moving a directory with an open child");
            assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}");
            assert_eq!(read_all(held_child), b"authorized immutable payload");
            fs::rename(&active, &parked).expect("rename must succeed after closing the child");
            fs::rename(&parked, &active).expect("restore the authorized directory");
        }

        assert_bounded_replacement_race(
            &io,
            &AssetPath::new("entry/payload.bin").unwrap(),
            &active,
            &alternate,
            [Some(b"authorized immutable payload"), None],
        );
        assert_eq!(
            fs::read(outside.path().join("payload.bin")).unwrap(),
            b"outside immutable payload"
        );
        #[cfg(windows)]
        fs::remove_dir(alternate).expect("remove the restored junction without following it");
    }

    #[test]
    fn symlink_replacement_race_never_reads_outside_bytes() {
        let root = TempRoot::new("symlink-race");
        let outside = TempRoot::new("symlink-race-outside");
        let active = root.path().join("entry.bin");
        let alternate = root.path().join("alternate.bin");
        fs::write(
            root.path().join("inside.bin"),
            b"authorized immutable payload",
        )
        .expect("write the authorized immutable file");
        fs::write(
            outside.path().join("payload.bin"),
            b"outside immutable payload",
        )
        .expect("write the outside immutable file");
        create_file_symlink(Path::new("inside.bin"), &active)
            .expect("create the relative in-root link");
        create_file_symlink(&outside.path().join("payload.bin"), &alternate)
            .expect("create the escaping file link");
        assert_eq!(fs::read(&alternate).unwrap(), b"outside immutable payload");
        let io = FileSystemAssetIo::from_capability(
            NativeDirectoryCapability::open_ambient(root.path()).expect("open race capability"),
        );
        assert!(io.open(&AssetPath::new("alternate.bin").unwrap()).is_err());

        assert_bounded_replacement_race(
            &io,
            &AssetPath::new("entry.bin").unwrap(),
            &active,
            &alternate,
            [Some(b"authorized immutable payload"), None],
        );
        assert_eq!(
            fs::read(outside.path().join("payload.bin")).unwrap(),
            b"outside immutable payload"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_fd_magic_link_cannot_reopen_an_outside_file() {
        use std::os::fd::AsRawFd;

        let outside = TempRoot::new("proc-fd-outside");
        let target = outside.path().join("payload.bin");
        fs::write(&target, b"outside immutable payload").expect("write the outside immutable file");
        let file =
            fs::File::open(&target).expect("retain the file referenced by the proc magic link");
        let name = file.as_raw_fd().to_string();
        let proc_path = Path::new("/proc/self/fd").join(&name);
        assert!(
            fs::symlink_metadata(&proc_path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&proc_path).unwrap(), b"outside immutable payload");
        let normal_io = FileSystemAssetIo::from_capability(
            NativeDirectoryCapability::open_ambient(outside.path())
                .expect("open ordinary capability"),
        );
        assert_eq!(
            read_all(
                normal_io
                    .open(&AssetPath::new("payload.bin").unwrap())
                    .unwrap()
            ),
            b"outside immutable payload"
        );
        let proc_io = FileSystemAssetIo::from_capability(
            NativeDirectoryCapability::open_ambient("/proc/self/fd")
                .expect("open proc fd capability"),
        );
        let error = proc_io
            .open(&AssetPath::new(&name).unwrap())
            .expect_err("a proc magic link must not escape its directory capability");
        assert_eq!(error.stage(), AssetOpenStage::Open);
        drop(file);
    }

    #[cfg(unix)]
    #[test]
    fn multi_link_chain_allows_in_root_parent_steps_and_rejects_escape() {
        let root = TempRoot::new("multi-link");
        let outside = TempRoot::new("multi-link-outside");
        fs::create_dir_all(root.path().join("links/nested"))
            .expect("create link-chain directories");
        fs::write(
            root.path().join("links/payload.bin"),
            b"authorized immutable payload",
        )
        .expect("write the authorized immutable file");
        fs::write(
            outside.path().join("payload.bin"),
            b"outside immutable payload",
        )
        .expect("write the outside immutable file");
        create_file_symlink(Path::new("links/first"), &root.path().join("entry"))
            .expect("create the first relative link");
        create_file_symlink(Path::new("nested/second"), &root.path().join("links/first"))
            .expect("create the second relative link");
        create_file_symlink(
            Path::new("../payload.bin"),
            &root.path().join("links/nested/second"),
        )
        .expect("create the in-root parent-relative final link");
        let escape = root.path().join("links/nested/escape");
        create_file_symlink(&outside.path().join("payload.bin"), &escape)
            .expect("precreate the escaping final link");
        let io = FileSystemAssetIo::from_capability(
            NativeDirectoryCapability::open_ambient(root.path())
                .expect("open link-chain capability"),
        );
        let request = AssetPath::new("entry").unwrap();
        assert_eq!(
            read_all(io.open(&request).unwrap()),
            b"authorized immutable payload"
        );
        assert!(
            io.open(&AssetPath::new("links/nested/escape").unwrap())
                .is_err()
        );

        fs::rename(escape, root.path().join("links/nested/second"))
            .expect("replace only the final symlink in the same multi-link chain");
        assert_eq!(
            fs::read(root.path().join("entry")).unwrap(),
            b"outside immutable payload"
        );
        assert!(io.open(&request).is_err(), "the full chain must not escape");
    }

    #[cfg(unix)]
    #[test]
    fn missing_directory_search_permission_returns_a_typed_failure() {
        use std::os::unix::fs::PermissionsExt;

        let root = TempRoot::new("search-permission");
        let restricted = root.path().join("restricted");
        fs::create_dir(&restricted).expect("create the restricted directory");
        let file = restricted.join("payload.bin");
        fs::write(&file, b"authorized immutable payload").expect("write permission fixture");
        let io = FileSystemAssetIo::from_capability(
            NativeDirectoryCapability::open_ambient(root.path()).expect("open root capability"),
        );
        let request = AssetPath::new("restricted/payload.bin").unwrap();
        assert_eq!(
            read_all(io.open(&request).unwrap()),
            b"authorized immutable payload"
        );
        let permissions = fs::metadata(&restricted).unwrap().permissions();
        fs::set_permissions(&restricted, fs::Permissions::from_mode(0o600))
            .expect("remove directory search permission");
        let host_result = fs::File::open(&file);
        let result = io.open(&request);
        // Restore the fixture before asserting; privileged runners must fail setup, never skip.
        fs::set_permissions(&restricted, permissions).expect("restore directory permissions");
        assert_eq!(
            host_result
                .expect_err("candidate runner must enforce search denial; use an unprivileged user")
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        let error = result.expect_err("capability resolution must preserve search denial");
        assert_eq!(error.stage(), AssetOpenStage::Open);
        assert_eq!(error.kind(), AssetOpenErrorKind::PermissionDenied);
        assert_eq!(
            read_all(io.open(&request).unwrap()),
            b"authorized immutable payload"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_case_alias_opens_only_the_authorized_in_root_file() {
        let root = TempRoot::new("case-alias");
        let outside = TempRoot::new("case-alias-outside");
        fs::write(
            root.path().join("MiXeD.bin"),
            b"authorized immutable payload",
        )
        .expect("write the authorized case-alias fixture");
        fs::write(
            outside.path().join("mixed.bin"),
            b"outside immutable payload",
        )
        .expect("write the outside same-name fixture");
        let io = FileSystemAssetIo::from_capability(
            NativeDirectoryCapability::open_ambient(root.path()).expect("open root capability"),
        );
        assert_eq!(
            read_all(io.open(&AssetPath::new("MiXeD.bin").unwrap()).unwrap()),
            b"authorized immutable payload"
        );
        for spelling in ["mixed.bin", "MIXED.BIN"] {
            assert_eq!(
                fs::read(root.path().join(spelling))
                    .expect("candidate runner must provide actual case aliases"),
                b"authorized immutable payload"
            );
            match io.open(&AssetPath::new(spelling).unwrap()) {
                Ok(opened) => assert_eq!(read_all(opened), b"authorized immutable payload"),
                Err(error) => assert_eq!(error.stage(), AssetOpenStage::Open),
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_firewall_rejects_ads_device_and_namespace_forms_before_open() {
        let cases = [
            (
                "payload.bin:stream",
                crate::PathErrorCode::AlternateDataStream,
            ),
            ("NUL.txt", crate::PathErrorCode::ReservedDevice),
            ("cOn.TxT", crate::PathErrorCode::ReservedDevice),
            ("COM1.log", crate::PathErrorCode::ReservedDevice),
            ("LPT\u{00b2}.bin", crate::PathErrorCode::ReservedDevice),
            ("CONIN$.txt", crate::PathErrorCode::ReservedDevice),
            ("PROGRA~1/asset.bin", crate::PathErrorCode::ShortNameAlias),
            ("ambiguous. ", crate::PathErrorCode::TrailingDotOrSpace),
            (r"C:relative", crate::PathErrorCode::DriveQualified),
            (r"C:\absolute", crate::PathErrorCode::DriveQualified),
            (r"\\server\share", crate::PathErrorCode::Absolute),
            (r"\\?\C:\secret", crate::PathErrorCode::Namespace),
            (r"\\.\NUL", crate::PathErrorCode::Namespace),
            (r"\??\C:\secret", crate::PathErrorCode::Namespace),
            (
                r"\Device\HarddiskVolume1\secret",
                crate::PathErrorCode::Namespace,
            ),
        ];

        for (candidate, expected) in cases {
            assert_eq!(
                AssetPath::new(candidate)
                    .expect_err("hostile path must fail")
                    .code(),
                expected,
                "unexpected classification for {candidate:?}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_junction_reparse_escape_is_rejected() {
        let root = TempRoot::new("junction-root");
        let outside = TempRoot::new("junction-outside");
        fs::write(outside.path().join("secret.bin"), b"outside").expect("write fixture");
        let junction = root.path().join("escape");
        create_junction(outside.path(), &junction)
            .expect("Windows candidate runner must create a junction fixture");

        let capability =
            NativeDirectoryCapability::open_ambient(root.path()).expect("open capability");
        let io = FileSystemAssetIo::from_capability(capability);
        let result = io.open(&AssetPath::new("escape/secret.bin").expect("valid path"));
        if let Err(error) = fs::remove_dir(&junction) {
            eprintln!("junction fixture cleanup deferred to temp-root cleanup: {error}");
        }

        assert!(result.is_err(), "junction escape unexpectedly opened");
        assert_eq!(
            fs::read(outside.path().join("secret.bin")).expect("outside fixture remains"),
            b"outside"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_unknown_reparse_payloads_never_become_parser_input() {
        use std::os::windows::fs::MetadataExt;

        let root = TempRoot::new("unknown-reparse");
        let fixture = std::process::Command::new("python")
            .arg("-c")
            .arg(include_str!("fs/tests/windows_reparse.py"))
            .arg(root.path())
            .output()
            .expect("Windows candidate runner must provide Python for native fixtures");
        assert!(
            fixture.status.success(),
            "unknown-tag fixture creation/readback failed: {}",
            String::from_utf8_lossy(&fixture.stderr)
        );
        for path in ["unknown.bin", "tagged", "tagged/unknown.bin"] {
            let metadata = fs::symlink_metadata(root.path().join(path))
                .expect("inspect the actual reparse fixture");
            assert_ne!(metadata.file_attributes() & 0x0000_0400, 0);
            assert!(!metadata.is_symlink(), "fixture must use an unknown tag");
        }
        let io = FileSystemAssetIo::from_capability(
            NativeDirectoryCapability::open_ambient(root.path()).expect("open capability"),
        );
        assert_eq!(
            read_all(io.open(&AssetPath::new("ordinary.bin").unwrap()).unwrap()),
            b"authorized ordinary file"
        );

        for path in ["unknown.bin", "tagged/unknown.bin"] {
            let error = io
                .open(&AssetPath::new(path).unwrap())
                .expect_err("an unresolved reparse payload must not be returned as a file");
            assert_eq!(error.stage(), AssetOpenStage::InspectObject);
            assert_eq!(
                error.kind(),
                AssetOpenErrorKind::NonRegularFile(AssetObjectKind::SymlinkOrReparsePoint)
            );
        }
        assert!(
            io.open(&AssetPath::new("tagged").unwrap()).is_err(),
            "a directory reparse object must not become parser input"
        );
        match io.open(&AssetPath::new("tagged/ordinary.bin").unwrap()) {
            Ok(file) => assert_eq!(read_all(file), b"authorized directory child"),
            Err(error) => assert!(matches!(
                error.stage(),
                AssetOpenStage::Open | AssetOpenStage::InspectObject
            )),
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_fifo_is_rejected_without_waiting_for_a_writer() {
        use std::process::Command;
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        let root = TempRoot::new("fifo");
        let fifo = root.path().join("input.fifo");
        let status = Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("Unix candidate runner must provide mkfifo");
        assert!(status.success(), "mkfifo failed with {status}");

        let capability =
            NativeDirectoryCapability::open_ambient(root.path()).expect("open capability");
        let io = FileSystemAssetIo::from_capability(capability);
        let path = AssetPath::new("input.fifo").expect("valid path");
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(io.open(&path));
        });

        let error = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("FIFO open must not block waiting for a writer")
            .expect_err("FIFO must not reach the parser");
        assert_eq!(error.stage(), AssetOpenStage::InspectObject);
        assert_eq!(
            error.kind(),
            AssetOpenErrorKind::NonRegularFile(AssetObjectKind::Fifo)
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_and_device_are_rejected() {
        use std::os::unix::net::UnixListener;

        let root = TempRoot::new("socket");
        let socket_path = root.path().join("input.socket");
        let _listener = UnixListener::bind(&socket_path).expect("bind socket fixture");
        let capability =
            NativeDirectoryCapability::open_ambient(root.path()).expect("open capability");
        let io = FileSystemAssetIo::from_capability(capability);
        let socket_error = io
            .open(&AssetPath::new("input.socket").expect("valid path"))
            .expect_err("socket must not reach the parser");
        assert!(matches!(
            socket_error.stage(),
            AssetOpenStage::Open | AssetOpenStage::InspectObject
        ));
        if socket_error.stage() == AssetOpenStage::InspectObject {
            assert_eq!(
                socket_error.kind(),
                AssetOpenErrorKind::NonRegularFile(AssetObjectKind::Socket)
            );
        }

        let device_capability = NativeDirectoryCapability::open_ambient("/dev")
            .expect("Unix candidate runner must provide a /dev directory capability");
        let device_io = FileSystemAssetIo::from_capability(device_capability);
        let device_error = device_io
            .open(&AssetPath::new("null").expect("valid path"))
            .expect_err("device must not reach the parser");
        assert_eq!(device_error.stage(), AssetOpenStage::InspectObject);
        assert_eq!(
            device_error.kind(),
            AssetOpenErrorKind::NonRegularFile(AssetObjectKind::CharacterDevice)
        );
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    #[cfg(windows)]
    fn create_junction(target: &Path, link: &Path) -> std::io::Result<()> {
        let output = std::process::Command::new("cmd.exe")
            .arg("/D")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ))
        }
    }

    #[cfg(not(windows))]
    fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }
}
