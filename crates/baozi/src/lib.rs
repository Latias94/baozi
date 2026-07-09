//! Public Baozi facade.

pub use baozi_core::*;
pub use baozi_import::{
    CapabilityStatus, FormatCapability, FormatInfo, FormatMaturity, ImporterRegistry,
    ReadConfidence,
};
pub use baozi_io::{AssetPath, AssetScope, AssetUri, FileSystemAssetIo, MemoryAssetIo};
pub use baozi_postprocess::{
    PostProcessPipeline, PostProcessPreset, PostProcessStage, PostProcessStep,
};

use baozi_import::ImporterRegistry as Registry;
use std::path::Path;

#[derive(Default)]
pub struct Importer {
    registry: Registry,
}

impl Importer {
    pub fn new() -> Self {
        let mut importer = Self {
            registry: Registry::new(),
        };
        importer.register_default_formats();
        importer
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn read_path(&self, path: impl AsRef<Path>) -> Result<Scene> {
        let path = path.as_ref();
        let hint = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        Err(BaoziError::unsupported_format(hint))
    }

    fn register_default_formats(&mut self) {
        #[cfg(feature = "format-stl")]
        baozi_format_stl::register(&mut self.registry);
        #[cfg(feature = "format-obj")]
        baozi_format_obj::register(&mut self.registry);
        #[cfg(feature = "format-ply")]
        baozi_format_ply::register(&mut self.registry);
        #[cfg(feature = "format-gltf")]
        baozi_format_gltf::register(&mut self.registry);
    }
}

pub fn load_scene(path: impl AsRef<Path>) -> Result<Scene> {
    Importer::new().read_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_reports_unsupported_until_parsers_exist() {
        let error = load_scene("model.unknown").unwrap_err();
        assert!(matches!(error, BaoziError::UnsupportedFormat { .. }));
    }
}
