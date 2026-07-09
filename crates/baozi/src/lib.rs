#![forbid(unsafe_code)]

//! Public Baozi facade.

pub use baozi_core::*;
pub use baozi_import::{
    CapabilityStatus, DetectionOptions, DiagnosticOptions, ExternalReferencePolicy,
    FormatCapability, FormatEncoding, FormatInfo, FormatMaturity, FormatSidecarPolicy,
    ImportOptions, ImportReport, ImportStage, ImportStats, IoOptions,
};
#[cfg(feature = "native-fs")]
pub use baozi_io::FileSystemAssetIo;
pub use baozi_io::{
    AssetIo, AssetPath, AssetScope, AssetUri, MemoryAssetIo, ReadSeek, ResourceLimits,
};
pub use baozi_postprocess::{
    PostProcessPipeline, PostProcessPreset, PostProcessStage, PostProcessStep,
};

use baozi_import::{FormatImporter, ImportContext, ImporterRegistry as Registry, ReadHint};
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

    pub fn register<I>(&mut self, importer: I) -> Result<()>
    where
        I: FormatImporter,
    {
        self.registry.register(importer)
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

    pub fn read_bytes_with_postprocess(
        &self,
        source: impl AsRef<str>,
        bytes: impl AsRef<[u8]>,
        options: ImportOptions,
        pipeline: &PostProcessPipeline,
    ) -> Result<ImportReport> {
        let report = self.read_bytes_with_options(source, bytes, options)?;
        Self::apply_postprocess(report, pipeline)
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
        ctx.into_report(scene, selected.info)
    }

    pub fn read_asset_with_postprocess(
        &self,
        io: &dyn AssetIo,
        source: AssetPath,
        options: ImportOptions,
        pipeline: &PostProcessPipeline,
    ) -> Result<ImportReport> {
        let report = self.read_asset_with_options(io, source, options)?;
        Self::apply_postprocess(report, pipeline)
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

    #[cfg(feature = "native-fs")]
    pub fn read_path_with_postprocess(
        &self,
        path: impl AsRef<Path>,
        options: ImportOptions,
        pipeline: &PostProcessPipeline,
    ) -> Result<ImportReport> {
        let report = self.read_path_with_options(path, options)?;
        Self::apply_postprocess(report, pipeline)
    }

    fn apply_postprocess(
        report: ImportReport,
        pipeline: &PostProcessPipeline,
    ) -> Result<ImportReport> {
        report.map_scene(ImportStage::PostProcessed, |scene| {
            let scene = pipeline.run(scene)?;
            validate_scene(&scene)?;
            Ok(scene)
        })
    }

    fn register_default_formats(&mut self) {
        #[cfg(feature = "format-stl")]
        baozi_format_stl::register(&mut self.registry).expect("built-in format ids must be unique");
        #[cfg(feature = "format-obj")]
        baozi_format_obj::register(&mut self.registry).expect("built-in format ids must be unique");
        #[cfg(feature = "format-ply")]
        baozi_format_ply::register(&mut self.registry).expect("built-in format ids must be unique");
        #[cfg(feature = "format-gltf")]
        baozi_format_gltf::register(&mut self.registry)
            .expect("built-in format ids must be unique");
    }
}

#[cfg(feature = "native-fs")]
pub fn load_scene(path: impl AsRef<Path>) -> Result<Scene> {
    Ok(Importer::new().read_path(path)?.into_scene())
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
