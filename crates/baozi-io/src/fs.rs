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
        let path_on_disk = self.scope.to_filesystem_path(path);
        let file = File::open(&path_on_disk)
            .map_err(|error| BaoziError::io(path.to_string(), error.to_string()))?;
        Ok(Box::new(file))
    }

    fn exists(&self, path: &AssetPath) -> bool {
        self.scope.to_filesystem_path(path).exists()
    }

    fn resolve(&self, base: &AssetPath, relative: &str) -> Result<AssetPath> {
        base.join(relative)
    }
}
