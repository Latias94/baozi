use crate::error::Result;
use baozi_core::FormatId;
use baozi_import::{
    DependencyResolver, FormatDescriptor, FormatImporter, FormatUnavailability,
    ImportOperationControl, ImportReport, ImportRequest, OptionCompileLimits,
    PreparedImportRequest, RegistryBuildLimits, RegistryBuilder, RegistrySnapshot, RequestError,
    import_asset, import_asset_with_resolver, import_bytes,
};
use baozi_io::{AssetIo, AssetPath};

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
    pub fn with_builtin_formats() -> Result<Self> {
        let mut builder = Self::new();
        builder.register_builtin_formats()?;
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

    fn register_builtin_formats(&mut self) -> Result<()> {
        #[cfg(feature = "format-stl")]
        baozi_format_stl::register(&mut self.registry)?;
        #[cfg(feature = "format-obj")]
        baozi_format_obj::register(&mut self.registry)?;
        #[cfg(feature = "format-ply")]
        baozi_format_ply::register(&mut self.registry)?;
        #[cfg(feature = "format-gltf")]
        baozi_format_gltf::register(&mut self.registry)?;
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
        ImporterBuilder::with_builtin_formats()?.build()
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
    pub fn builder_with_builtin_formats() -> Result<ImporterBuilder> {
        ImporterBuilder::with_builtin_formats()
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
