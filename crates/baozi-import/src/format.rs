use crate::context::{ImportContext, ReadHint};
use baozi_core::{Result, Scene};
use baozi_io::ReadSeek;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormatMaturity {
    Experimental,
    Beta,
    Stable,
    Deprecated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilityStatus {
    Supported,
    Partial,
    ParsedLossy,
    IgnoredWithDiagnostic,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormatCapability {
    Geometry,
    Hierarchy,
    Materials,
    Textures,
    CamerasLights,
    Animation,
    Skinning,
    MorphTargets,
    Metadata,
    CompressionContainers,
    CoordinatesUnits,
    Diagnostics,
    ResourceLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormatEncoding {
    Text,
    Binary,
    TextOrBinary,
    Archive,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormatSidecarPolicy {
    None,
    Optional,
    Required,
    ArchiveEntries,
    ExternalBuffers,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatInfo {
    id: &'static str,
    display_name: &'static str,
    extensions: &'static [&'static str],
    media_types: &'static [&'static str],
    encoding: FormatEncoding,
    sidecar_policy: FormatSidecarPolicy,
    maturity: FormatMaturity,
    capabilities: &'static [(FormatCapability, CapabilityStatus)],
    notes: &'static str,
    docs_path: Option<&'static str>,
}

impl FormatInfo {
    pub const fn new(
        id: &'static str,
        display_name: &'static str,
        extensions: &'static [&'static str],
    ) -> Self {
        Self {
            id,
            display_name,
            extensions,
            media_types: &[],
            encoding: FormatEncoding::Unknown,
            sidecar_policy: FormatSidecarPolicy::Unknown,
            maturity: FormatMaturity::Experimental,
            capabilities: &[],
            notes: "",
            docs_path: None,
        }
    }

    pub const fn with_media_types(mut self, media_types: &'static [&'static str]) -> Self {
        self.media_types = media_types;
        self
    }

    pub const fn with_encoding(mut self, encoding: FormatEncoding) -> Self {
        self.encoding = encoding;
        self
    }

    pub const fn with_sidecar_policy(mut self, sidecar_policy: FormatSidecarPolicy) -> Self {
        self.sidecar_policy = sidecar_policy;
        self
    }

    pub const fn with_maturity(mut self, maturity: FormatMaturity) -> Self {
        self.maturity = maturity;
        self
    }

    pub const fn with_capabilities(
        mut self,
        capabilities: &'static [(FormatCapability, CapabilityStatus)],
    ) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub const fn with_notes(mut self, notes: &'static str) -> Self {
        self.notes = notes;
        self
    }

    pub const fn with_docs_path(mut self, docs_path: &'static str) -> Self {
        self.docs_path = Some(docs_path);
        self
    }

    pub const fn id(&self) -> &'static str {
        self.id
    }

    pub const fn display_name(&self) -> &'static str {
        self.display_name
    }

    pub const fn extensions(&self) -> &'static [&'static str] {
        self.extensions
    }

    pub const fn media_types(&self) -> &'static [&'static str] {
        self.media_types
    }

    pub const fn encoding(&self) -> FormatEncoding {
        self.encoding
    }

    pub const fn sidecar_policy(&self) -> FormatSidecarPolicy {
        self.sidecar_policy
    }

    pub const fn maturity(&self) -> FormatMaturity {
        self.maturity
    }

    pub const fn capabilities(&self) -> &'static [(FormatCapability, CapabilityStatus)] {
        self.capabilities
    }

    pub const fn notes(&self) -> &'static str {
        self.notes
    }

    pub const fn docs_path(&self) -> Option<&'static str> {
        self.docs_path
    }

    pub fn capability_status(&self, capability: FormatCapability) -> Option<CapabilityStatus> {
        self.capabilities
            .iter()
            .find_map(|(candidate, status)| (*candidate == capability).then_some(*status))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReadConfidence {
    No,
    Maybe,
    Likely,
    Certain,
}

pub trait FormatImporter: Send + Sync + 'static {
    fn info(&self) -> FormatInfo;

    fn can_read(&self, _input: &mut dyn ReadSeek, _hint: &ReadHint) -> Result<ReadConfidence> {
        Ok(ReadConfidence::No)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> Result<Scene>;
}
