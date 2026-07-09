use baozi_core::{BaoziError, Result, Scene};
use baozi_import::{
    CapabilityStatus, FormatCapability, FormatImporter, FormatInfo, FormatMaturity, ImportContext,
    ImporterRegistry,
};

pub struct GltfImporter;

pub fn format_info() -> FormatInfo {
    FormatInfo {
        id: "gltf",
        display_name: "glTF 2.0",
        extensions: &["gltf", "glb"],
        maturity: FormatMaturity::Experimental,
        capabilities: &[
            (FormatCapability::Geometry, CapabilityStatus::Unknown),
            (FormatCapability::Materials, CapabilityStatus::Unknown),
            (FormatCapability::Textures, CapabilityStatus::Unknown),
            (FormatCapability::Animation, CapabilityStatus::Unknown),
            (FormatCapability::Skinning, CapabilityStatus::Unknown),
        ],
        notes: "planned glTF importer shell; parsing is not implemented yet",
    }
}

pub fn register(registry: &mut ImporterRegistry) {
    registry.register(GltfImporter);
}

impl FormatImporter for GltfImporter {
    fn info(&self) -> FormatInfo {
        format_info()
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> Result<Scene> {
        Err(BaoziError::unsupported_format(
            "gltf parser not implemented",
        ))
    }
}
