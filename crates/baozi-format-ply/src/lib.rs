use baozi_core::{BaoziError, Result, Scene};
use baozi_import::{
    CapabilityStatus, FormatCapability, FormatImporter, FormatInfo, FormatMaturity, ImportContext,
    ImporterRegistry,
};

pub struct PlyImporter;

pub fn format_info() -> FormatInfo {
    FormatInfo {
        id: "ply",
        display_name: "PLY",
        extensions: &["ply"],
        maturity: FormatMaturity::Experimental,
        capabilities: &[
            (FormatCapability::Geometry, CapabilityStatus::Unknown),
            (FormatCapability::Metadata, CapabilityStatus::Unknown),
        ],
        notes: "planned PLY importer shell; parsing is not implemented yet",
    }
}

pub fn register(registry: &mut ImporterRegistry) {
    registry.register(PlyImporter);
}

impl FormatImporter for PlyImporter {
    fn info(&self) -> FormatInfo {
        format_info()
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> Result<Scene> {
        Err(BaoziError::unsupported_format("ply parser not implemented"))
    }
}
