use baozi_core::{BaoziError, Result, Scene};
use baozi_import::{
    CapabilityStatus, FormatCapability, FormatImporter, FormatInfo, FormatMaturity, ImportContext,
    ImporterRegistry,
};

pub struct StlImporter;

pub fn format_info() -> FormatInfo {
    FormatInfo {
        id: "stl",
        display_name: "STL",
        extensions: &["stl"],
        maturity: FormatMaturity::Experimental,
        capabilities: &[(FormatCapability::Geometry, CapabilityStatus::Unknown)],
        notes: "planned STL importer shell; parsing is not implemented yet",
    }
}

pub fn register(registry: &mut ImporterRegistry) {
    registry.register(StlImporter);
}

impl FormatImporter for StlImporter {
    fn info(&self) -> FormatInfo {
        format_info()
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> Result<Scene> {
        Err(BaoziError::unsupported_format("stl parser not implemented"))
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
