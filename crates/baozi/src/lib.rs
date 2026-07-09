//! Public Baozi facade.

pub use baozi_core::*;
pub use baozi_import::{
    CapabilityStatus, DetectionOptions, DiagnosticOptions, ExternalReferencePolicy,
    FormatCapability, FormatImporter, FormatInfo, FormatMaturity, ImportContext, ImportOptions,
    ImportReport, ImporterRegistry, IoOptions, ReadConfidence, ReadHint,
};
#[cfg(feature = "native-fs")]
pub use baozi_io::FileSystemAssetIo;
pub use baozi_io::{
    AssetIo, AssetPath, AssetScope, AssetUri, MemoryAssetIo, ReadSeek, ResourceLimits,
};
pub use baozi_postprocess::{
    PostProcessPipeline, PostProcessPreset, PostProcessStage, PostProcessStep,
};

use baozi_import::ImporterRegistry as Registry;
use std::sync::Arc;

#[cfg(feature = "native-fs")]
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

    pub fn empty() -> Self {
        Self {
            registry: Registry::new(),
        }
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut Registry {
        &mut self.registry
    }

    pub fn register<I>(&mut self, importer: I)
    where
        I: FormatImporter,
    {
        self.registry.register(importer);
    }

    pub fn read_bytes(
        &self,
        source: impl AsRef<str>,
        bytes: impl AsRef<[u8]>,
    ) -> Result<ImportReport> {
        self.read_bytes_with_options(source, bytes, ImportOptions::memory())
    }

    pub fn read_bytes_with_options(
        &self,
        source: impl AsRef<str>,
        bytes: impl AsRef<[u8]>,
        options: ImportOptions,
    ) -> Result<ImportReport> {
        let bytes = bytes.as_ref();
        if bytes.len() as u64 > options.limits.max_primary_asset_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_primary_asset_bytes",
            });
        }

        let source = AssetPath::new(source.as_ref())?;
        let mut io = MemoryAssetIo::new();
        io.insert(source.clone(), Arc::<[u8]>::from(bytes));
        self.read_asset_with_options(&io, source, options)
    }

    pub fn read_asset(&self, io: &dyn AssetIo, source: AssetPath) -> Result<ImportReport> {
        self.read_asset_with_options(io, source, ImportOptions::memory())
    }

    pub fn read_asset_with_options(
        &self,
        io: &dyn AssetIo,
        source: AssetPath,
        options: ImportOptions,
    ) -> Result<ImportReport> {
        let mut input = io.open(&source)?;
        let hint = ReadHint::from_source(source.clone());
        let selected = self
            .registry
            .detect_with_options(&mut *input, &hint, &options.detection)?;
        drop(input);

        let mut ctx = ImportContext::with_options(io, source, options);
        let scene = selected.importer.read(&mut ctx)?;
        Ok(ctx.into_report(scene, selected.info))
    }

    #[cfg(feature = "native-fs")]
    pub fn read_path(&self, path: impl AsRef<Path>) -> Result<ImportReport> {
        self.read_path_with_options(path, ImportOptions::filesystem())
    }

    #[cfg(feature = "native-fs")]
    pub fn read_path_with_options(
        &self,
        path: impl AsRef<Path>,
        options: ImportOptions,
    ) -> Result<ImportReport> {
        let path = path.as_ref();
        let file_name = path
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .ok_or_else(|| {
                BaoziError::io(path.display().to_string(), "asset path has no file name")
            })?;
        let root = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let io = FileSystemAssetIo::new(AssetScope::new(root.to_path_buf()));
        self.read_asset_with_options(&io, AssetPath::new(file_name)?, options)
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

#[cfg(feature = "native-fs")]
pub fn load_scene(path: impl AsRef<Path>) -> Result<Scene> {
    Ok(Importer::new().read_path(path)?.scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_reports_unsupported_until_parsers_exist() {
        let error = Importer::new()
            .read_bytes("model.unknown", b"not a supported model")
            .unwrap_err();
        assert!(matches!(error, BaoziError::UnsupportedFormat { .. }));
    }
}
