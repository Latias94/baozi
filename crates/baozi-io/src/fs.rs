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

fn classify_object(file_type: cap_std::fs::FileType) -> AssetObjectKind {
    if file_type.is_dir() {
        return AssetObjectKind::Directory;
    }
    if file_type.is_symlink() {
        return AssetObjectKind::SymlinkOrReparsePoint;
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
