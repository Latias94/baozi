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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub extensions: &'static [&'static str],
    pub maturity: FormatMaturity,
    pub capabilities: &'static [(FormatCapability, CapabilityStatus)],
    pub notes: &'static str,
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
        Ok(ReadConfidence::Maybe)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> Result<Scene>;
}
