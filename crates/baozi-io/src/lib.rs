//! Runtime-neutral asset IO contracts for Baozi importers.

#[cfg(feature = "fs")]
pub mod fs;
pub mod limits;
pub mod memory;
pub mod path;

#[cfg(feature = "fs")]
pub use fs::FileSystemAssetIo;
pub use limits::ResourceLimits;
pub use memory::MemoryAssetIo;
pub use path::{AssetPath, AssetScope, AssetUri};

use baozi_core::Result;
use std::io::{Read, Seek};

pub trait ReadSeek: Read + Seek {}

impl<T> ReadSeek for T where T: Read + Seek {}

pub trait AssetIo: Send + Sync {
    fn open(&self, path: &AssetPath) -> Result<Box<dyn ReadSeek + Send>>;
    fn exists(&self, path: &AssetPath) -> bool;
    fn resolve(&self, base: &AssetPath, relative: &str) -> Result<AssetPath>;
}
