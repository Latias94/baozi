use crate::{AssetIo, AssetPath, ReadSeek};
use baozi_core::{BaoziError, Result};
use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct MemoryAssetIo {
    assets: BTreeMap<AssetPath, Arc<[u8]>>,
}

impl MemoryAssetIo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, path: AssetPath, bytes: impl Into<Arc<[u8]>>) {
        self.assets.insert(path, bytes.into());
    }
}

impl AssetIo for MemoryAssetIo {
    fn open(&self, path: &AssetPath) -> Result<Box<dyn ReadSeek + Send>> {
        let bytes = self
            .assets
            .get(path)
            .cloned()
            .ok_or_else(|| BaoziError::io(path.to_string(), "memory asset not found"))?;
        Ok(Box::new(Cursor::new(bytes)))
    }

    fn exists(&self, path: &AssetPath) -> bool {
        self.assets.contains_key(path)
    }

    fn resolve(&self, base: &AssetPath, relative: &str) -> Result<AssetPath> {
        base.join(relative)
    }
}
