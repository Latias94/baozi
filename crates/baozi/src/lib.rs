#![forbid(unsafe_code)]

//! Public Baozi facade.

pub use baozi_core::*;
pub use baozi_import::{
    CapabilityStatus, DetectionEvidence, DetectionFailure, DetectionFault, DetectionInconclusive,
    DetectionOptions, DetectionOutcome, DiagnosticOptions, ExternalReferencePolicy,
    FormatCapability, FormatEncoding, FormatInfo, FormatMaturity, FormatSidecarPolicy,
    ImportFailure, ImportOptions, ImportReport, ImportResult, ImportStage, ImportStats, IoOptions,
    PrimaryProbeOperation, ReadConfidence,
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
use std::any::Any;
use std::sync::Arc;

pub type Result<T> = ImportResult<T>;

#[cfg(feature = "native-fs")]
use std::path::Path;

pub struct Importer {
    registry: Registry,
}

impl Default for Importer {
    fn default() -> Self {
        Self::new()
    }
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

    pub fn register<I>(&mut self, importer: I) -> baozi_core::Result<()>
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
            }
            .into());
        }
        if bytes.len() as u64 > options.limits.max_total_asset_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_total_asset_bytes",
            }
            .into());
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
            .detect_with_options(&mut *input, &hint, &options.detection)
            .map_err(ImportFailure::from_detection)?;
        drop(input);

        let (format_importer, format, _confidence, detection_evidence) = selected.into_parts();

        let source_name = source.to_string();
        let mut ctx = ImportContext::with_options(io, source, options);
        let scene = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            format_importer.read(&mut ctx)
        }))
        .map_err(|panic| {
            BaoziError::parse(
                source_name,
                None,
                format!(
                    "format importer {} panicked: {}",
                    format.id(),
                    panic_message(panic.as_ref())
                ),
            )
        })
        .and_then(|result| result)
        .map_err(|error| ImportFailure::operation(error, detection_evidence.clone()))?;
        ctx.finish_import(scene, format, detection_evidence.clone())
            .map_err(|error| ImportFailure::operation(error, detection_evidence))
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
        report.apply_postprocess(|scene| pipeline.run(scene))
    }

    fn register_default_formats(&mut self) {
        #[cfg(feature = "format-stl")]
        baozi_format_stl::register(&mut self.registry).expect("built-in format ids must be unique");
        #[cfg(feature = "format-obj")]
        baozi_format_obj::register(&mut self.registry).expect("built-in format ids must be unique");
        #[cfg(feature = "format-ply")]
        baozi_format_ply::register(&mut self.registry).expect("built-in format ids must be unique");
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&'static str>().copied())
        .unwrap_or("unknown panic payload")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_content_returns_typed_unsupported_format() {
        let error = Importer::new()
            .read_bytes("model.unknown", b"not a supported model")
            .unwrap_err();
        assert!(matches!(error, ImportFailure::UnsupportedFormat { .. }));
    }
}
