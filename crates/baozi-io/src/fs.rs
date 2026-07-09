use crate::{AssetIo, AssetPath, AssetScope, ReadSeek};
use baozi_core::{BaoziError, Result};
use std::fs::File;

#[derive(Debug, Clone)]
pub struct FileSystemAssetIo {
    scope: AssetScope,
}

impl FileSystemAssetIo {
    pub fn new(scope: AssetScope) -> Self {
        Self { scope }
    }
}

impl AssetIo for FileSystemAssetIo {
    fn open(&self, path: &AssetPath) -> Result<Box<dyn ReadSeek + Send>> {
        let path_on_disk = self.scope.resolve_existing(path)?;
        let file = File::open(&path_on_disk)
            .map_err(|error| BaoziError::io(path.to_string(), error.to_string()))?;
        Ok(Box::new(file))
    }

    fn exists(&self, path: &AssetPath) -> bool {
        self.scope.resolve_existing(path).is_ok()
    }

    fn resolve(&self, base: &AssetPath, relative: &str) -> Result<AssetPath> {
        base.join(relative)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Read;
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

    #[test]
    fn opens_file_inside_scope() {
        let root = TempRoot::new("inside-scope");
        fs::write(root.path().join("mesh.stl"), b"solid test").unwrap();
        let io = FileSystemAssetIo::new(AssetScope::new(root.path()));

        let mut reader = io.open(&AssetPath::new("mesh.stl").unwrap()).unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();

        assert_eq!(bytes, b"solid test");
        assert!(io.exists(&AssetPath::new("mesh.stl").unwrap()));
    }

    #[test]
    fn rejects_symlink_escape_when_platform_allows_symlink_creation() {
        let root = TempRoot::new("symlink-root");
        let outside = TempRoot::new("symlink-outside");
        fs::write(outside.path().join("secret.stl"), b"secret").unwrap();

        let link = root.path().join("link.stl");
        if create_file_symlink(&outside.path().join("secret.stl"), &link).is_err() {
            return;
        }

        let io = FileSystemAssetIo::new(AssetScope::new(root.path()));
        let error = match io.open(&AssetPath::new("link.stl").unwrap()) {
            Ok(_) => panic!("symlink escape unexpectedly opened"),
            Err(error) => error,
        };

        assert!(matches!(error, BaoziError::Io { .. }));
        assert!(!io.exists(&AssetPath::new("link.stl").unwrap()));
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    #[cfg(not(windows))]
    fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }
}
