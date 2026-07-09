#![forbid(unsafe_code)]

mod ascii;
mod binary;
mod detect;
mod parser;

use baozi_core::{Result, Scene};
use baozi_import::{
    CapabilityStatus, FormatCapability, FormatImporter, FormatInfo, FormatMaturity, ImportContext,
    ImporterRegistry, ReadConfidence, ReadHint,
};
use baozi_io::ReadSeek;

pub struct StlImporter;

pub fn format_info() -> FormatInfo {
    FormatInfo {
        id: "stl",
        display_name: "STL",
        extensions: &["stl"],
        maturity: FormatMaturity::Experimental,
        capabilities: &[
            (FormatCapability::Geometry, CapabilityStatus::Supported),
            (FormatCapability::Hierarchy, CapabilityStatus::Partial),
            (FormatCapability::Materials, CapabilityStatus::Partial),
            (FormatCapability::Metadata, CapabilityStatus::Partial),
            (
                FormatCapability::CoordinatesUnits,
                CapabilityStatus::ParsedLossy,
            ),
            (FormatCapability::Diagnostics, CapabilityStatus::Supported),
            (
                FormatCapability::ResourceLimits,
                CapabilityStatus::Supported,
            ),
        ],
        notes: "experimental STL importer for binary and ASCII triangle meshes",
    }
}

pub fn register(registry: &mut ImporterRegistry) {
    registry.register(StlImporter);
}

impl FormatImporter for StlImporter {
    fn info(&self) -> FormatInfo {
        format_info()
    }

    fn can_read(&self, input: &mut dyn ReadSeek, hint: &ReadHint) -> Result<ReadConfidence> {
        detect::can_read(input, hint)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> Result<Scene> {
        parser::read_stl(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_experimental_maturity() {
        assert_eq!(format_info().maturity, FormatMaturity::Experimental);
    }
}
