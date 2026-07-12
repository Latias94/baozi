#![forbid(unsafe_code)]

//! Public Baozi facade.
//!
//! Native filesystem helpers are candidate-only under `native-fs-candidate`; no release
//! `native-fs` capability is currently registered.

pub use baozi_core::{
    Aabb, AlphaMode, Animation, AnimationChannel, AnimationId, AnimationInterpolation,
    AnimationProperty, AnimationTarget, AnimationValues, Camera, CameraId, CameraProjection, Color,
    ColorSpace, FormatId, FormatIdError, Light, LightId, LightKind, Mat4, Material, MaterialId,
    MaterialProperty, MaterialPropertyMap, Mesh, MeshBinding, MeshId, MetadataKeyViolation,
    MetadataMap, MetadataValue, MetadataValueViolation, MorphTarget, Node, NodeId,
    PrimitiveTopology, Quat, SceneCanonicalCounts, SceneCollection, SceneFootprint,
    SceneGenerationId, SceneMetadataOwner, SceneSpace, SceneStorageFootprint, SceneStringOwner,
    SceneStringViolation, SceneValidationError, SceneValidationLimit, SceneValidationStats, Skin,
    SkinId, Texture, TextureFilterMode, TextureId, TextureRole, TextureSampler, TextureSlot,
    TextureSource, TextureTransform, TextureWrapMode, ValidatedScene, Vec2, Vec3, Vec4,
    VertexAttribute, VertexAttributeData, VertexAttributeSemantic,
};
pub use baozi_import::{
    CapabilityStatus, CauseOwner, ConformancePolicy, DependencyPolicyError, DependencyRequirement,
    DependencyResolver, DependencyRoleId, DetectionEvidence, DetectionFailure, DetectionFault,
    DetectionFaultCode, DetectionInconclusive, DetectionOptions, DetectionOptionsError,
    DetectionOutcome, DiagnosticCause, DiagnosticCode, DiagnosticSeverity, DiagnosticSummary,
    FormatCapability, FormatDescriptor, FormatEncoding, FormatFailureDetail,
    FormatFailureDetailValue, FormatFailureEnvelope, FormatImporter, FormatMaturity,
    FormatOptionEnvelope, FormatOptionEnvelopeError, FormatSelection, FormatUnavailability,
    ImportCause, ImportConfig, ImportConfigOverlay, ImportDiagnostic, ImportDimension,
    ImportFailure, ImportLedgerPhase, ImportLedgerSnapshot, ImportLossKind, ImportOperationControl,
    ImportPhase, ImportPhaseEvidence, ImportPreset, ImportProgressPhase, ImportProgressUnit,
    ImportReport, ImportRequest, ImportRequestBuildError, ImportRequestLayer, ImportRunError,
    LossPolicy, OptionCompileLimits, PreparedImportRequest, ReadConfidence, RegistryBuildError,
    RegistryBuildLimits, RegistryRegistrationError, RequestError, ResourceLimits,
    ResourceLimitsOverlay, ScopeAbort, ScopeAbortCause, TrustedExtensionBoundary,
};
pub use baozi_io::{
    AssetIo, AssetLength, AssetObjectKind, AssetOpenError, AssetOpenErrorKind, AssetOpenStage,
    AssetPath, AssetReadContractViolation, AssetReadError, AssetReadErrorKind, AuthorityDomain,
    MemoryAssetInsertError, MemoryAssetIo, NATIVE_FS_CANDIDATE_TARGETS,
    NATIVE_FS_REGISTERED_TARGETS, NATIVE_FS_SUPPORT_STATUS, NativeFsSupportStatus, OpenedAsset,
    PathError, PathErrorCode,
};
#[cfg(feature = "native-fs-candidate")]
pub use baozi_io::{FileSystemAssetIo, NativeDirectoryCapability};
pub use baozi_operation::{
    CancellationOrigin, CancellationSignal, CancellationToken, ControlCause, LedgerLimitsError,
    ProgressDecision, ProgressDeliveryLimits, ProgressEvent, ProgressObserver,
    ProgressStorageError, ProgressViolation,
};
pub use baozi_postprocess::{
    PostProcessControl, PostProcessDimension, PostProcessError, PostProcessFailure,
    PostProcessLedgerPhase, PostProcessLedgerSnapshot, PostProcessLimits, PostProcessPipeline,
    PostProcessPlanError, PostProcessPreset, PostProcessProgressPhase, PostProcessProgressUnit,
    PostProcessRunError, PostProcessStep, PostProcessSuccess,
};

use baozi_import::{
    RegistryBuilder, RegistrySnapshot, import_asset, import_asset_with_resolver, import_bytes,
};
use thiserror::Error;

#[cfg(feature = "native-fs-candidate")]
use std::path::Path;

/// A facade-owned failure that preserves the typed lower-layer cause.
#[derive(Debug, Error)]
#[non_exhaustive]
#[allow(
    clippy::large_enum_variant,
    reason = "facade import failures retain complete bounded terminal evidence without failure-path allocation"
)]
pub enum Error {
    #[error(transparent)]
    RegistryRegistration(#[from] RegistryRegistrationError),

    #[error(transparent)]
    RegistryBuild(#[from] RegistryBuildError),

    #[error(transparent)]
    Request(#[from] RequestError),

    #[error(transparent)]
    Path(#[from] PathError),

    #[error(transparent)]
    MemoryAssetInsert(#[from] MemoryAssetInsertError),

    #[cfg(feature = "native-fs-candidate")]
    #[error("native directory capability could not be opened")]
    NativeRoot(#[source] AssetOpenError),

    #[cfg(feature = "native-fs-candidate")]
    #[error("native source path has no terminal file name")]
    MissingFileName,

    #[cfg(feature = "native-fs-candidate")]
    #[error("native source file name is not valid UTF-8")]
    NonUtf8FileName,

    #[error("import operation initialization failed: {0}")]
    ImportInitialization(#[source] LedgerLimitsError<ImportDimension>),

    #[error(transparent)]
    Import(#[from] ImportFailure),
}

impl From<ImportRunError> for Error {
    fn from(error: ImportRunError) -> Self {
        match error.into_operation_failure() {
            Ok(failure) => Self::Import(failure),
            Err(source) => Self::ImportInitialization(source),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
pub type ProcessedImportResult<T> = std::result::Result<T, ProcessedImportFailure>;

/// One successful import followed by a distinct successful post-process operation.
#[derive(Debug)]
pub struct ProcessedImport {
    import: ImportReport,
    postprocess: PostProcessSuccess,
}

impl ProcessedImport {
    pub const fn import(&self) -> &ImportReport {
        &self.import
    }

    pub const fn postprocess(&self) -> &PostProcessSuccess {
        &self.postprocess
    }

    pub const fn scene(&self) -> &ValidatedScene {
        self.postprocess.scene()
    }

    pub fn into_parts(self) -> (ImportReport, PostProcessSuccess) {
        (self.import, self.postprocess)
    }
}

/// A composed workflow failure that retains a completed import when post-processing fails.
#[derive(Debug, Error)]
#[non_exhaustive]
#[allow(
    clippy::large_enum_variant,
    reason = "composed failures retain either complete import evidence or the completed import report by value"
)]
pub enum ProcessedImportFailure {
    #[error(transparent)]
    Import(#[from] Error),

    #[error("post-process failed after import succeeded")]
    PostProcess {
        import: ImportReport,
        #[source]
        failure: PostProcessRunError,
    },
}

impl ProcessedImportFailure {
    pub fn completed_import(&self) -> Option<&ImportReport> {
        match self {
            Self::Import(_) => None,
            Self::PostProcess { import, .. } => Some(import),
        }
    }

    pub fn postprocess_failure(&self) -> Option<&PostProcessFailure> {
        match self {
            Self::Import(_) => None,
            Self::PostProcess {
                failure: PostProcessRunError::Operation { failure },
                ..
            } => Some(failure),
            Self::PostProcess { .. } => None,
        }
    }

    pub fn postprocess_run_error(&self) -> Option<&PostProcessRunError> {
        match self {
            Self::Import(_) => None,
            Self::PostProcess { failure, .. } => Some(failure),
        }
    }
}

/// Mutable registry proposal that can publish one immutable [`Importer`].
pub struct ImporterBuilder {
    registry: RegistryBuilder,
    limits: RegistryBuildLimits,
}

#[allow(
    clippy::result_large_err,
    reason = "facade results preserve complete bounded terminal evidence without heap indirection"
)]
impl ImporterBuilder {
    /// Creates an empty registry proposal.
    pub fn new() -> Self {
        Self {
            registry: RegistryBuilder::new(),
            limits: RegistryBuildLimits::default(),
        }
    }

    /// Creates a proposal populated by every enabled first-party format.
    pub fn with_default_formats() -> Result<Self> {
        let mut builder = Self::new();
        builder.register_default_formats()?;
        Ok(builder)
    }

    pub fn with_registry_build_limits(mut self, limits: RegistryBuildLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn register<I>(&mut self, importer: I) -> Result<&mut Self>
    where
        I: FormatImporter,
    {
        self.registry.register(importer)?;
        Ok(self)
    }

    pub fn declare_unavailable(
        &mut self,
        id: FormatId,
        reason: FormatUnavailability,
    ) -> Result<&mut Self> {
        self.registry.declare_unavailable(id, reason)?;
        Ok(self)
    }

    pub fn build(self) -> Result<Importer> {
        Ok(Importer {
            registry: self.registry.freeze(self.limits)?,
        })
    }

    fn register_default_formats(&mut self) -> Result<()> {
        #[cfg(feature = "format-stl")]
        baozi_format_stl::register(&mut self.registry)?;
        #[cfg(feature = "format-obj")]
        baozi_format_obj::register(&mut self.registry)?;
        #[cfg(feature = "format-ply")]
        baozi_format_ply::register(&mut self.registry)?;
        Ok(())
    }
}

impl Default for ImporterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Immutable, reentrant facade over one frozen format registry.
pub struct Importer {
    registry: RegistrySnapshot,
}

impl std::fmt::Debug for Importer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Importer")
            .field("formats", &self.registry)
            .finish()
    }
}

#[allow(
    clippy::result_large_err,
    reason = "facade results preserve complete bounded terminal evidence without heap indirection"
)]
impl Importer {
    /// Freezes every enabled first-party format under the default build limits.
    pub fn new() -> Result<Self> {
        ImporterBuilder::with_default_formats()?.build()
    }

    /// Freezes a registry containing no formats.
    pub fn empty() -> Result<Self> {
        ImporterBuilder::new().build()
    }

    /// Starts an empty custom registry proposal.
    pub fn builder() -> ImporterBuilder {
        ImporterBuilder::new()
    }

    /// Starts a custom proposal containing every enabled first-party format.
    pub fn builder_with_default_formats() -> Result<ImporterBuilder> {
        ImporterBuilder::with_default_formats()
    }

    pub fn formats(&self) -> impl ExactSizeIterator<Item = &FormatDescriptor> {
        self.registry.descriptors()
    }

    pub fn format(&self, id: &FormatId) -> Option<&FormatDescriptor> {
        self.registry.descriptor(id)
    }

    /// Compiles caller-submitted format envelopes before any session or asset IO exists.
    pub fn prepare_request(
        &self,
        request: &ImportRequest,
        limits: OptionCompileLimits,
    ) -> Result<PreparedImportRequest> {
        Ok(self.registry.compile_request(request, limits)?)
    }

    /// Compiles the empty option request against cached registry defaults.
    pub fn prepare_default_request(&self) -> Result<PreparedImportRequest> {
        self.prepare_request(&ImportRequest::new(), OptionCompileLimits::default())
    }

    pub fn read_bytes(
        &self,
        source: impl AsRef<str>,
        bytes: impl AsRef<[u8]>,
    ) -> Result<ImportReport> {
        let request = self.prepare_default_request()?;
        self.read_bytes_with_request(source, bytes, &request, ImportOperationControl::default())
    }

    pub fn read_bytes_with_request(
        &self,
        source: impl AsRef<str>,
        bytes: impl AsRef<[u8]>,
        request: &PreparedImportRequest,
        control: ImportOperationControl,
    ) -> Result<ImportReport> {
        self.validate_request(request)?;
        let source = AssetPath::new(source.as_ref())?;
        Ok(import_bytes(request, source, bytes.as_ref(), control)?)
    }

    pub fn read_asset(&self, io: &dyn AssetIo, source: AssetPath) -> Result<ImportReport> {
        let request = self.prepare_default_request()?;
        self.read_asset_with_request(io, source, &request, ImportOperationControl::default())
    }

    pub fn read_asset_with_request(
        &self,
        io: &dyn AssetIo,
        source: AssetPath,
        request: &PreparedImportRequest,
        control: ImportOperationControl,
    ) -> Result<ImportReport> {
        self.validate_request(request)?;
        Ok(import_asset(request, io, source, control)?)
    }

    pub fn read_asset_with_resolver(
        &self,
        io: &dyn AssetIo,
        source: AssetPath,
        request: &PreparedImportRequest,
        resolver: &dyn DependencyResolver,
        control: ImportOperationControl,
    ) -> Result<ImportReport> {
        self.validate_request(request)?;
        Ok(import_asset_with_resolver(
            request, io, source, resolver, control,
        )?)
    }

    fn validate_request(&self, request: &PreparedImportRequest) -> Result<()> {
        if request.belongs_to(&self.registry) {
            Ok(())
        } else {
            Err(RequestError::RegistrySnapshotMismatch.into())
        }
    }

    pub fn process(
        report: ImportReport,
        pipeline: &PostProcessPipeline,
    ) -> ProcessedImportResult<ProcessedImport> {
        Self::process_with_control(report, pipeline, PostProcessControl::default())
    }

    pub fn process_with_control(
        report: ImportReport,
        pipeline: &PostProcessPipeline,
        control: PostProcessControl,
    ) -> ProcessedImportResult<ProcessedImport> {
        match pipeline.run_with_control(report.scene(), control) {
            Ok(postprocess) => Ok(ProcessedImport {
                import: report,
                postprocess,
            }),
            Err(failure) => Err(ProcessedImportFailure::PostProcess {
                import: report,
                failure,
            }),
        }
    }

    pub fn read_bytes_with_postprocess(
        &self,
        source: impl AsRef<str>,
        bytes: impl AsRef<[u8]>,
        pipeline: &PostProcessPipeline,
    ) -> ProcessedImportResult<ProcessedImport> {
        let report = self.read_bytes(source, bytes)?;
        Self::process(report, pipeline)
    }

    pub fn read_asset_with_postprocess(
        &self,
        io: &dyn AssetIo,
        source: AssetPath,
        pipeline: &PostProcessPipeline,
    ) -> ProcessedImportResult<ProcessedImport> {
        let report = self.read_asset(io, source)?;
        Self::process(report, pipeline)
    }

    #[cfg(feature = "native-fs-candidate")]
    /// Imports from a native path through the unregistered candidate adapter.
    pub fn read_path(&self, path: impl AsRef<Path>) -> Result<ImportReport> {
        let request = self.prepare_default_request()?;
        self.read_path_with_request(path, &request, ImportOperationControl::default())
    }

    #[cfg(feature = "native-fs-candidate")]
    /// Imports from a native path with an explicit request through the candidate adapter.
    pub fn read_path_with_request(
        &self,
        path: impl AsRef<Path>,
        request: &PreparedImportRequest,
        control: ImportOperationControl,
    ) -> Result<ImportReport> {
        self.validate_request(request)?;
        let path = path.as_ref();
        let file_name = path.file_name().ok_or(Error::MissingFileName)?;
        let file_name = file_name.to_str().ok_or(Error::NonUtf8FileName)?;
        let root = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let capability =
            NativeDirectoryCapability::open_ambient(root).map_err(Error::NativeRoot)?;
        let io = FileSystemAssetIo::from_capability(capability);
        self.read_asset_with_request(&io, AssetPath::new(file_name)?, request, control)
    }

    #[cfg(feature = "native-fs-candidate")]
    /// Imports and post-processes a native path through the candidate adapter.
    pub fn read_path_with_postprocess(
        &self,
        path: impl AsRef<Path>,
        pipeline: &PostProcessPipeline,
    ) -> ProcessedImportResult<ProcessedImport> {
        let report = self.read_path(path)?;
        Self::process(report, pipeline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_is_frozen_and_has_no_formats() {
        let importer = Importer::empty().expect("empty registry must freeze");
        assert_eq!(importer.formats().len(), 0);
    }
}
