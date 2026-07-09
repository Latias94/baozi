#![forbid(unsafe_code)]

mod detect;
mod mesh_builder;
mod mtl;
mod obj;
mod parser;

use baozi_core::{Result, Scene};
use baozi_import::{
    CapabilityStatus, FormatCapability, FormatImporter, FormatInfo, FormatMaturity, ImportContext,
    ImporterRegistry, ReadConfidence, ReadHint,
};
use baozi_io::ReadSeek;

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

    fn can_read(&self, input: &mut dyn ReadSeek, hint: &ReadHint) -> Result<ReadConfidence> {
        detect::can_read(input, hint)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> Result<Scene> {
        parser::read_obj(ctx)
    }
}
