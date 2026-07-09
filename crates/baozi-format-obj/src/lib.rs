use baozi_core::{BaoziError, Result, Scene};
use baozi_import::{
    CapabilityStatus, FormatCapability, FormatImporter, FormatInfo, FormatMaturity, ImportContext,
    ImporterRegistry,
};

pub struct ObjImporter;

pub fn format_info() -> FormatInfo {
    FormatInfo {
        id: "obj",
        display_name: "Wavefront OBJ",
        extensions: &["obj"],
        maturity: FormatMaturity::Experimental,
        capabilities: &[
            (FormatCapability::Geometry, CapabilityStatus::Unknown),
            (FormatCapability::Materials, CapabilityStatus::Unknown),
            (FormatCapability::Textures, CapabilityStatus::Unknown),
        ],
        notes: "planned OBJ/MTL importer shell; parsing is not implemented yet",
    }
}

pub fn register(registry: &mut ImporterRegistry) {
    registry.register(ObjImporter);
}

impl FormatImporter for ObjImporter {
    fn info(&self) -> FormatInfo {
        format_info()
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> Result<Scene> {
        Err(BaoziError::unsupported_format("obj parser not implemented"))
    }
}
